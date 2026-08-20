//! ProofForge gate tools over MCP.
//!
//! Deliberately narrow surface — the ship lane only:
//! `pf_doctor` (toolchain sanity) → `pf_check` (semantic gate) →
//! `pf_build` (sealed artifacts) → `pf_artifacts` (inspect closure).
//! Network broadcast and key material are out of scope by construction;
//! deployment happens in the ProofShip app with its wallet stack.

use crate::cli::{CliResult, run_cli};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use std::time::Duration;

const CHECK_TIMEOUT: Duration = Duration::from_secs(600);
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckParams {
    /// Path to the ProgramV1 `.lean` source file (absolute, or relative to the session cwd).
    pub source: String,
    /// Module name declared in the source (`module <Name> v1`).
    pub module: String,
    /// Optional project root passed through as `--root`.
    #[serde(default)]
    pub root: Option<String>,
    /// Override the default timeout (seconds).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuildParams {
    /// Path to the ProgramV1 `.lean` source file.
    pub source: String,
    /// Module name declared in the source.
    pub module: String,
    /// Target backend id. Defaults to `evm`.
    #[serde(default)]
    pub target: Option<String>,
    /// Output directory for the sealed artifact set (`-o`).
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Optional project root passed through as `--root`.
    #[serde(default)]
    pub root: Option<String>,
    /// Optional build profile (`--profile`).
    #[serde(default)]
    pub profile: Option<String>,
    /// Override the default timeout (seconds).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactsParams {
    /// Inspect a build output directory (artifact closure). Preferred form.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Or inspect a registry target descriptor by id.
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoctorParams {
    /// Limit the report to specific target ids.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Include runtime checks.
    #[serde(default)]
    pub with_runtime: bool,
    /// Report every known target, not just core ones.
    #[serde(default)]
    pub include_all: bool,
}

#[derive(Clone, Default)]
pub struct PfGate;

fn wrap(result: CliResult) -> CallToolResult {
    let text = vec![ContentBlock::text(result.to_text())];
    if result.ok {
        CallToolResult::success(text)
    } else {
        CallToolResult::error(text)
    }
}

fn timeout_or(default: Duration, seconds: Option<u64>) -> Duration {
    seconds.map(Duration::from_secs).unwrap_or(default)
}

#[tool_router]
impl PfGate {
    pub fn new() -> Self {
        Self
    }

    #[tool(
        name = "pf_check",
        description = "Gate a ProofForge ProgramV1 source: parse + semantic checks (moves, guards, capabilities). Returns the CLI JSON wrap; on failure `parsed.diagnostics[]` carries machine-readable codes for the repair loop. Must pass before pf_build."
    )]
    async fn pf_check(
        &self,
        Parameters(p): Parameters<CheckParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.source.trim().is_empty() || p.module.trim().is_empty() {
            return Ok(wrap(CliResult::usage(
                "pf_check requires source and module",
            )));
        }
        let mut argv = vec![
            "check".into(),
            p.source.clone(),
            "--module".into(),
            p.module.clone(),
            "--json".into(),
        ];
        if let Some(root) = p.root.as_deref().filter(|s| !s.trim().is_empty()) {
            argv.extend(["--root".into(), root.into()]);
        }
        Ok(wrap(
            run_cli(&argv, timeout_or(CHECK_TIMEOUT, p.timeout_seconds)).await,
        ))
    }

    #[tool(
        name = "pf_build",
        description = "Compile a gate-passed ProgramV1 source into a sealed artifact set (bytecode + ABI + digest + gate report). Target defaults to `evm`. This tool never broadcasts to a network and never touches keys — deployment is done in the Waku app."
    )]
    async fn pf_build(
        &self,
        Parameters(p): Parameters<BuildParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.source.trim().is_empty() || p.module.trim().is_empty() {
            return Ok(wrap(CliResult::usage(
                "pf_build requires source and module",
            )));
        }
        let target = p
            .target
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("evm");
        let mut argv = vec![
            "build".into(),
            p.source.clone(),
            "--module".into(),
            p.module.clone(),
            "--target".into(),
            target.into(),
            "--json".into(),
        ];
        if let Some(out) = p.output_dir.as_deref().filter(|s| !s.trim().is_empty()) {
            argv.extend(["-o".into(), out.into()]);
        }
        if let Some(root) = p.root.as_deref().filter(|s| !s.trim().is_empty()) {
            argv.extend(["--root".into(), root.into()]);
        }
        if let Some(profile) = p.profile.as_deref().filter(|s| !s.trim().is_empty()) {
            argv.extend(["--profile".into(), profile.into()]);
        }
        Ok(wrap(
            run_cli(&argv, timeout_or(BUILD_TIMEOUT, p.timeout_seconds)).await,
        ))
    }

    #[tool(
        name = "pf_artifacts",
        description = "Inspect a sealed artifact set: pass `outputDir` (a pf_build output directory) to verify the closure and read the outputSetDigest, or `target` to read a registry target descriptor."
    )]
    async fn pf_artifacts(
        &self,
        Parameters(p): Parameters<ArtifactsParams>,
    ) -> Result<CallToolResult, McpError> {
        let argv: Vec<String> =
            if let Some(dir) = p.output_dir.as_deref().filter(|s| !s.trim().is_empty()) {
                // Path form is forced so a target-id collision cannot hijack inspect.
                vec![
                    "inspect".into(),
                    "--output-dir".into(),
                    dir.into(),
                    "--json".into(),
                ]
            } else if let Some(target) = p.target.as_deref().filter(|s| !s.trim().is_empty()) {
                vec!["inspect".into(), target.into(), "--json".into()]
            } else {
                return Ok(wrap(CliResult::usage(
                    "pf_artifacts requires outputDir (build output) or target (registry inspect)",
                )));
            };
        Ok(wrap(run_cli(&argv, INSPECT_TIMEOUT).await))
    }

    #[tool(
        name = "pf_doctor",
        description = "Report ProofForge toolchain health (locked compiler versions per target). Exit code 3 (missing/partial toolchain) is informational, not a tool error."
    )]
    async fn pf_doctor(
        &self,
        Parameters(p): Parameters<DoctorParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut argv = vec!["doctor".into(), "--json".into()];
        for t in &p.targets {
            if !t.trim().is_empty() {
                argv.extend(["--target".into(), t.clone()]);
            }
        }
        if p.with_runtime {
            argv.push("--with-runtime".into());
        }
        if p.include_all {
            argv.push("--all".into());
        }
        let mut result = run_cli(&argv, INSPECT_TIMEOUT).await;
        // Doctor exit 3 = missing/partial toolchain; the JSON body is the answer.
        if result.parsed.is_some() && matches!(result.exit_code, Some(0) | Some(3)) {
            result.ok = true;
            result.error = None;
        }
        Ok(wrap(result))
    }
}

#[tool_handler(
    name = "proofship-pf-mcp",
    version = "1.0.0",
    instructions = "ProofForge gate for ProgramV1 contracts. Flow: pf_doctor (once, sanity) -> pf_check until the gate passes -> pf_build for sealed artifacts -> pf_artifacts to verify the closure/digest. Results are a JSON wrap {ok, exitCode, stdout, stderr, parsed, error}; read parsed.diagnostics[] on failures and fix codes in order. Never ask this server to deploy: use Waku's session Deploy flow."
)]
impl ServerHandler for PfGate {}
