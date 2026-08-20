//! Detect the ProofForge gate binary and attach MCP servers to providers.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

use super::okx::{ACCESS_KEY_HEADER, ONCHAINOS_MCP_NAME, ONCHAINOS_MCP_URL};

const PF_MCP_BIN: &str = "proofship-pf-mcp";

#[derive(Clone, Debug, Default)]
pub struct Web3McpAttachment {
    pub pf_mcp: Option<PathBuf>,
    pub okx_key: Option<String>,
}

pub fn detect_attachment() -> Web3McpAttachment {
    Web3McpAttachment {
        pf_mcp: if env_truthy("WAKU_DISABLE_PF_MCP") || env_truthy("PROOFSHIP_DISABLE_PF_MCP") {
            None
        } else {
            detect_pf_mcp()
        },
        okx_key: super::okx::access_key(),
    }
}

fn detect_pf_mcp() -> Option<PathBuf> {
    if !toolchain_signal()
        && env_path("WAKU_PF_MCP").is_none()
        && env_path("PROOFSHIP_PF_MCP").is_none()
    {
        return None;
    }
    env_path("WAKU_PF_MCP")
        .or_else(|| env_path("PROOFSHIP_PF_MCP"))
        .or_else(bundled_gate)
        .or_else(|| find_on_path(PF_MCP_BIN))
        .or_else(|| find_on_path("proofship-pf-mcp"))
}

fn toolchain_signal() -> bool {
    super::pf_cli().is_some()
        || env_path("PF_CLI").is_some()
        || env_path("PROOF_FORGE_CLI").is_some()
        || find_on_path("proof-forge-next").is_some()
        || std::env::var("PROOF_FORGE_ROOT")
            .ok()
            .map(PathBuf::from)
            .is_some_and(|root| root.join(".lake/build/bin/proof-forge-next").is_file())
        || std::env::var("PROOFSHIP_PF_MCP_URL")
            .ok()
            .is_some_and(|url| !url.trim().is_empty())
}

fn pf_env_json() -> serde_json::Map<String, Value> {
    let mut env = serde_json::Map::new();
    for (key, value) in super::pf_cli_env() {
        env.insert(key, json!(value));
    }
    env
}

fn bundled_gate() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let sibling = parent.join(format!("{PF_MCP_BIN}{}", std::env::consts::EXE_SUFFIX));
    sibling.is_file().then_some(sibling)
}

pub fn attach_codex_mcp(command: &mut Command) {
    let attachment = detect_attachment();
    if let Some(bin) = attachment.pf_mcp {
        command
            .arg("-c")
            .arg(format!("mcp_servers.waku_pf_mcp.command={}", bin.display()));
        for (key, value) in super::pf_cli_env() {
            command
                .arg("-c")
                .arg(format!("mcp_servers.waku_pf_mcp.env.{key}={value}"));
        }
    }
    if let Some(key) = attachment.okx_key {
        command
            .arg("-c")
            .arg(format!(
                "mcp_servers.{ONCHAINOS_MCP_NAME}.url={ONCHAINOS_MCP_URL}"
            ))
            .arg("-c")
            .arg(format!(
                "mcp_servers.{ONCHAINOS_MCP_NAME}.http_headers.{ACCESS_KEY_HEADER}={key}"
            ));
    }
    crate::ship::attach_codex_catalog(command, &crate::ship::enabled_catalog());
}

pub fn merge_opencode_mcp(mcp: &mut serde_json::Map<String, Value>) {
    let attachment = detect_attachment();
    if let Some(bin) = attachment.pf_mcp {
        let mut server = json!({
            "type": "local",
            "command": [bin.display().to_string()],
            "enabled": true,
        });
        let env = pf_env_json();
        if !env.is_empty() {
            server["environment"] = Value::Object(env);
        }
        mcp.insert("waku_pf_mcp".into(), server);
    }
    if let Some(key) = attachment.okx_key {
        mcp.insert(
            ONCHAINOS_MCP_NAME.replace('-', "_").into(),
            json!({
                "type": "remote",
                "url": ONCHAINOS_MCP_URL,
                "enabled": true,
                "headers": { ACCESS_KEY_HEADER: key },
            }),
        );
    }
    crate::ship::merge_opencode_catalog(mcp, &crate::ship::enabled_catalog());
}

pub fn merge_grok_mcp(mcp_servers: &mut toml::Table) {
    let attachment = detect_attachment();
    if let Some(bin) = attachment.pf_mcp {
        let mut server = toml::Table::new();
        server.insert(
            "command".into(),
            toml::Value::String(bin.display().to_string()),
        );
        let env_pairs = super::pf_cli_env();
        if !env_pairs.is_empty() {
            let mut env = toml::Table::new();
            for (key, value) in env_pairs {
                env.insert(key, toml::Value::String(value));
            }
            server.insert("env".into(), toml::Value::Table(env));
        }
        mcp_servers.insert("waku_pf_mcp".into(), toml::Value::Table(server));
    }
    if let Some(key) = attachment.okx_key {
        let mut headers = toml::Table::new();
        headers.insert(ACCESS_KEY_HEADER.into(), toml::Value::String(key));
        let mut server = toml::Table::new();
        server.insert("url".into(), toml::Value::String(ONCHAINOS_MCP_URL.into()));
        server.insert("http_headers".into(), toml::Value::Table(headers));
        mcp_servers.insert(
            ONCHAINOS_MCP_NAME.replace('-', "_"),
            toml::Value::Table(server),
        );
    }
    crate::ship::merge_grok_catalog(mcp_servers, &crate::ship::enabled_catalog());
}

pub fn attach_claude_mcp(command: &mut Command) {
    let Some(path) = write_claude_mcp_config() else {
        return;
    };
    command.args(["--mcp-config", &path.to_string_lossy()]);
}

fn write_claude_mcp_config() -> Option<PathBuf> {
    let attachment = detect_attachment();
    let catalog = crate::ship::enabled_catalog();
    if attachment.pf_mcp.is_none() && attachment.okx_key.is_none() && catalog.is_empty() {
        return None;
    }
    let mut mcp_servers = serde_json::Map::new();
    if let Some(bin) = attachment.pf_mcp {
        let mut server = json!({
            "command": bin,
            "args": []
        });
        let env = pf_env_json();
        if !env.is_empty() {
            server["env"] = Value::Object(env);
        }
        mcp_servers.insert("proofship-pf-mcp".into(), server);
    }
    if let Some(key) = attachment.okx_key {
        mcp_servers.insert(
            ONCHAINOS_MCP_NAME.into(),
            json!({
                "type": "http",
                "url": ONCHAINOS_MCP_URL,
                "headers": { ACCESS_KEY_HEADER: key }
            }),
        );
    }
    crate::ship::merge_claude_catalog(&mut mcp_servers, &catalog);
    if mcp_servers.is_empty() {
        return None;
    }
    let config = Value::Object({
        let mut root = serde_json::Map::new();
        root.insert("mcpServers".into(), Value::Object(mcp_servers));
        root
    });
    let dir = std::env::temp_dir().join("waku-web3");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("claude-mcp.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&config).ok()?).ok()?;
    Some(path)
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let executable = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&paths)
        .map(|dir| dir.join(&executable))
        .find(|path| path.is_file())
}

fn env_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}
