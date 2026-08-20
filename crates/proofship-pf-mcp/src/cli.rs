//! Locate and run the `proof-forge-next` product CLI.
//!
//! Every tool result is the same JSON wrap the old python MCP-V0 server
//! emitted (`{ ok, exitCode, command, stdout, stderr, parsed, error }`) so
//! agents that learned the shape keep working.

use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

pub const SCHEMA_WRAP: &str = "pf.mcp.result.v1";

/// Hard cap for stdout/stderr echoed back to the model. Lean diagnostics can
/// be huge; keep the tail, which carries the actual error.
const STREAM_CAP: usize = 32 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliResult {
    pub schema: &'static str,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub command: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    pub parsed: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl CliResult {
    pub fn usage(message: &str) -> Self {
        Self {
            schema: SCHEMA_WRAP,
            ok: false,
            exit_code: Some(2),
            command: Vec::new(),
            stdout: String::new(),
            stderr: message.to_string(),
            parsed: None,
            error: Some("usage".into()),
        }
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!("{{\"ok\":false,\"stderr\":\"serialize: {e}\"}}"))
    }
}

/// Resolve the product CLI: `PROOF_FORGE_CLI` override, then the lake build
/// output under `PROOF_FORGE_ROOT`, then `PATH`.
pub fn find_cli() -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("PROOF_FORGE_CLI") {
        let v = v.trim();
        if !v.is_empty() {
            let p = expand_home(v);
            if p.is_file() {
                return Ok(p);
            }
            return Err(format!("PROOF_FORGE_CLI is not a file: {}", p.display()));
        }
    }
    if let Ok(root) = std::env::var("PROOF_FORGE_ROOT") {
        let root = root.trim();
        if !root.is_empty() {
            let built = expand_home(root).join(".lake/build/bin/proof-forge-next");
            if built.is_file() {
                return Ok(built);
            }
        }
    }
    if let Some(found) = which("proof-forge-next") {
        return Ok(found);
    }
    if let Ok(root) = std::env::var("WAKU_PROOF_FORGE_ROOT") {
        let root = root.trim();
        if !root.is_empty() {
            let built = expand_home(root).join("bin/proof-forge-next");
            if built.is_file() {
                return Ok(built);
            }
        }
    }
    Err("proof-forge-next not found; set PROOF_FORGE_CLI or PROOF_FORGE_ROOT (build with `lake build`), or add it to PATH".into())
}

pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Run the CLI with a timeout, capturing both streams. Never inherits stdin.
pub async fn run_cli(args: &[String], timeout: Duration) -> CliResult {
    let cli = match find_cli() {
        Ok(p) => p,
        Err(e) => {
            return CliResult {
                schema: SCHEMA_WRAP,
                ok: false,
                exit_code: None,
                command: args.to_vec(),
                stdout: String::new(),
                stderr: e,
                parsed: None,
                error: Some("toolchain-missing".into()),
            };
        }
    };
    let mut command: Vec<String> = vec![cli.display().to_string()];
    command.extend(args.iter().cloned());

    let mut cmd = tokio::process::Command::new(&cli);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Lake-based CLI resolves its own root; run from PROOF_FORGE_ROOT when set
    // so registry/target lookups behave like a checkout invocation.
    if let Ok(root) = std::env::var("PROOF_FORGE_ROOT") {
        let root = root.trim();
        if !root.is_empty() {
            let dir = expand_home(root);
            if dir.is_dir() {
                cmd.current_dir(dir);
            }
        }
    }

    let out = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return CliResult {
                schema: SCHEMA_WRAP,
                ok: false,
                exit_code: None,
                command,
                stdout: String::new(),
                stderr: format!("spawn failed: {e}"),
                parsed: None,
                error: Some("failed".into()),
            };
        }
        Err(_) => {
            return CliResult {
                schema: SCHEMA_WRAP,
                ok: false,
                exit_code: None,
                command,
                stdout: String::new(),
                stderr: format!("timeout after {}s", timeout.as_secs()),
                parsed: None,
                error: Some("timeout".into()),
            };
        }
    };

    let stdout = tail_utf8(&out.stdout);
    let stderr = tail_utf8(&out.stderr);
    let code = out.status.code();
    let ok = out.status.success();
    let parsed = try_parse_json(&stdout);
    CliResult {
        schema: SCHEMA_WRAP,
        ok,
        exit_code: code,
        command,
        stdout,
        stderr: stderr.clone(),
        parsed,
        error: if ok {
            None
        } else {
            Some(classify_error(&stderr, code))
        },
    }
}

fn tail_utf8(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= STREAM_CAP {
        return text.into_owned();
    }
    let cut = text.len() - STREAM_CAP;
    let boundary = text
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= cut)
        .unwrap_or(cut);
    format!("…[truncated {boundary} bytes]…{}", &text[boundary..])
}

/// Prefer whole stdout as JSON; otherwise try the outermost `{...}` block so
/// trailing log noise does not hide a JSON body.
fn try_parse_json(text: &str) -> Option<serde_json::Value> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str(s) {
        return Some(v);
    }
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

/// Map product error markers / exit codes onto the stable classes agents key
/// their repair loops off.
fn classify_error(stderr: &str, code: Option<i32>) -> String {
    if stderr.contains("PF-TOOLCHAIN-MISSING") {
        return "toolchain-missing".into();
    }
    if stderr.contains("PF-TOOLCHAIN-MISMATCH") {
        return "toolchain-mismatch".into();
    }
    if stderr.contains("PF-SRC-INVALID") {
        return "src-invalid".into();
    }
    if stderr.contains("PF-OUTPUT-MANIFEST") {
        return "output-manifest".into();
    }
    match code {
        Some(2) => "usage".into(),
        Some(3) => "product-error".into(),
        _ => "failed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_with_noise() {
        let v = try_parse_json("warming up...\n{\"ok\":true}\n").unwrap();
        assert_eq!(v["ok"], serde_json::Value::Bool(true));
        assert!(try_parse_json("plain text").is_none());
    }

    #[test]
    fn error_classes() {
        assert_eq!(
            classify_error("PF-SRC-INVALID: bad", Some(3)),
            "src-invalid"
        );
        assert_eq!(classify_error("boom", Some(2)), "usage");
        assert_eq!(classify_error("boom", Some(3)), "product-error");
        assert_eq!(classify_error("boom", Some(1)), "failed");
    }

    #[test]
    fn wrap_serializes_camel_case() {
        let text = CliResult::usage("missing module").to_text();
        assert!(text.contains("\"exitCode\":2"));
        assert!(text.contains("\"error\":\"usage\""));
    }
}
