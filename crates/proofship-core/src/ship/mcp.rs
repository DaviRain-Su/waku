//! User-managed MCP catalog. OAuth is the agent's job; no tokens in headers.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};
use proofship_protocol::ship::{McpServer, McpTransport, builtin_mcp_servers, is_builtin_mcp_id};

const FILE_MODE: u32 = 0o600;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogFile {
    #[serde(default)]
    servers: Vec<McpServer>,
}

#[derive(Clone)]
pub struct McpCatalog {
    file: PathBuf,
}

impl McpCatalog {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("mcp.json"),
        }
    }

    fn read(&self) -> CatalogFile {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn write(&self, catalog: &CatalogFile) -> anyhow::Result<()> {
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(catalog)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(FILE_MODE))?;
        }
        std::fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    pub fn load(&self) -> anyhow::Result<Vec<McpServer>> {
        let existed = self.file.exists();
        let mut servers = self.read().servers;
        let mut dirty = !existed;
        for builtin in builtin_mcp_servers() {
            if servers.iter().any(|server| server.id == builtin.id) {
                continue;
            }
            let insert_at = servers
                .iter()
                .position(|server| !server.builtin)
                .unwrap_or(servers.len());
            servers.insert(insert_at, builtin);
            dirty = true;
        }
        if dirty {
            self.write(&CatalogFile {
                servers: servers.clone(),
            })?;
        }
        Ok(servers)
    }

    pub fn upsert(&self, mut server: McpServer) -> anyhow::Result<Vec<McpServer>> {
        server.name = server.name.trim().to_string();
        if server.name.is_empty() {
            bail!("name is required");
        }
        if server.id.trim().is_empty() {
            server.id = format!("mcp-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        }
        if is_builtin_mcp_id(&server.id) {
            bail!("built-in MCP rows can be toggled, not edited");
        }
        match server.transport {
            McpTransport::Http => {
                let url = normalize_url(server.url.as_deref().unwrap_or(""))?;
                server.url = Some(url);
                server.command = None;
                server.args.clear();
            }
            McpTransport::Stdio => {
                let command = server
                    .command
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("command is required"))?;
                server.command = Some(command.to_string());
                server.url = None;
            }
        }
        server.builtin = false;
        server.source = Some("catalog".into());
        server.auth.clear();
        server.auth_account = None;
        let mut servers = self.load()?;
        if let Some(existing) = servers.iter_mut().find(|row| row.id == server.id) {
            if existing.builtin {
                bail!("built-in MCP rows can be toggled, not edited");
            }
            *existing = server;
        } else {
            servers.push(server);
        }
        self.write(&CatalogFile {
            servers: servers.clone(),
        })?;
        Ok(servers)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<Vec<McpServer>> {
        let mut servers = self.load()?;
        let Some(row) = servers.iter_mut().find(|server| server.id == id) else {
            bail!("mcp server not found");
        };
        row.enabled = enabled;
        self.write(&CatalogFile {
            servers: servers.clone(),
        })?;
        Ok(servers)
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<Vec<McpServer>> {
        let mut servers = self.load()?;
        let Some(index) = servers.iter().position(|server| server.id == id) else {
            bail!("mcp server not found");
        };
        if servers[index].builtin {
            bail!("built-in MCP rows can be toggled, not removed");
        }
        servers.remove(index);
        self.write(&CatalogFile {
            servers: servers.clone(),
        })?;
        Ok(servers)
    }

    pub fn enabled(&self) -> Vec<McpServer> {
        if env_truthy("WAKU_DISABLE_USER_MCP") {
            return Vec::new();
        }
        self.load()
            .unwrap_or_default()
            .into_iter()
            .filter(|server| server.enabled)
            .collect()
    }
}

fn normalize_url(raw: &str) -> anyhow::Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("URL is required");
    }
    let rest = if let Some(rest) = raw.strip_prefix("https://") {
        rest
    } else if let Some(rest) = raw.strip_prefix("http://") {
        rest
    } else if let Some(rest) = raw.strip_prefix("HTTPS://") {
        rest
    } else if let Some(rest) = raw.strip_prefix("HTTP://") {
        rest
    } else {
        bail!("MCP URL must be http or https");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        bail!("MCP URL is missing a host");
    }
    Ok(raw.to_string())
}

fn env_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_builtins_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpCatalog::new(dir.path());
        let servers = store.load().unwrap();
        assert!(servers.iter().any(|server| server.id == "github"));
        assert!(servers.iter().any(|server| server.id == "cloudflare-api"));
        assert!(servers.iter().any(|server| server.id == "vercel"));
        assert!(servers.iter().all(|server| !server.enabled));
    }

    #[test]
    fn cannot_remove_or_edit_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpCatalog::new(dir.path());
        store.load().unwrap();
        assert!(store.remove("github").is_err());
        assert!(
            store
                .upsert(McpServer {
                    id: "github".into(),
                    name: "Nope".into(),
                    transport: McpTransport::Http,
                    url: Some("https://example.com".into()),
                    command: None,
                    args: Vec::new(),
                    enabled: true,
                    builtin: false,
                    source: None,
                    ..Default::default()
                })
                .is_err()
        );
        let servers = store.set_enabled("github", true).unwrap();
        assert!(
            servers
                .iter()
                .any(|server| server.id == "github" && server.enabled)
        );
    }

    #[test]
    fn custom_http_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpCatalog::new(dir.path());
        let servers = store
            .upsert(McpServer {
                id: String::new(),
                name: "Docs".into(),
                transport: McpTransport::Http,
                url: Some("https://example.com/mcp".into()),
                command: None,
                args: Vec::new(),
                enabled: true,
                builtin: false,
                source: None,
                ..Default::default()
            })
            .unwrap();
        assert!(
            servers
                .iter()
                .any(|server| server.name == "Docs" && server.enabled)
        );
    }
}
