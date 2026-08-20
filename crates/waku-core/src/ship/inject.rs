//! Attach the user MCP catalog and Web2 skills to every new provider run.

use std::process::Command;

use serde_json::{json, Value};
use waku_protocol::ship::{McpServer, McpTransport};

use super::mcp::McpCatalog;

const CLOUDFLARE_MARKER: &str = "<!-- waku:cloudflare -->";
const VERCEL_MARKER: &str = "<!-- waku:vercel -->";
const CLOUDFLARE_SKILL: &str =
    include_str!("../../../../resources/skills/waku-cloudflare/SKILL.md");
const VERCEL_SKILL: &str = include_str!("../../../../resources/skills/waku-vercel/SKILL.md");

pub fn catalog_servers(catalog: &McpCatalog) -> Vec<McpServer> {
    catalog.enabled()
}

pub fn cloudflare_enabled(servers: &[McpServer]) -> bool {
    servers.iter().any(|server| {
        server.enabled
            && server
                .url
                .as_deref()
                .is_some_and(|url| url.contains("mcp.cloudflare.com") || url.contains("bindings.mcp.cloudflare.com") || url.contains("docs.mcp.cloudflare.com"))
    })
}

pub fn vercel_enabled(servers: &[McpServer]) -> bool {
    servers.iter().any(|server| {
        server.enabled && server.url.as_deref().is_some_and(|url| url.contains("mcp.vercel.com"))
    })
}

pub fn enrich_prompt(mut prompt: String, servers: &[McpServer]) -> String {
    if cloudflare_enabled(servers) && !prompt.contains(CLOUDFLARE_MARKER) {
        prompt = format!(
            "{CLOUDFLARE_MARKER}\n{}\n\n{prompt}",
            strip_yaml_frontmatter(CLOUDFLARE_SKILL)
        );
    }
    if vercel_enabled(servers) && !prompt.contains(VERCEL_MARKER) {
        prompt = format!(
            "{VERCEL_MARKER}\n{}\n\n{prompt}",
            strip_yaml_frontmatter(VERCEL_SKILL)
        );
    }
    prompt
}

pub fn attach_codex_catalog(command: &mut Command, servers: &[McpServer]) {
    for server in servers {
        let name = sanitize_id(&server.id);
        match server.transport {
            McpTransport::Http => {
                if let Some(url) = &server.url {
                    command
                        .arg("-c")
                        .arg(format!("mcp_servers.{name}.url={url}"));
                    if let Some(token) = super::mcp_access_token(&server.id) {
                        command.arg("-c").arg(format!(
                            "mcp_servers.{name}.http_headers.Authorization=Bearer {token}"
                        ));
                    }
                }
            }
            McpTransport::Stdio => {
                if let Some(bin) = &server.command {
                    command
                        .arg("-c")
                        .arg(format!("mcp_servers.{name}.command={bin}"));
                    if !server.args.is_empty() {
                        let args = server
                            .args
                            .iter()
                            .map(|arg| format!("\"{arg}\""))
                            .collect::<Vec<_>>()
                            .join(",");
                        command
                            .arg("-c")
                            .arg(format!("mcp_servers.{name}.args=[{args}]"));
                    }
                }
            }
        }
    }
}

pub fn merge_opencode_catalog(mcp: &mut serde_json::Map<String, Value>, servers: &[McpServer]) {
    for server in servers {
        let name = sanitize_id(&server.id);
        match server.transport {
            McpTransport::Http => {
                if let Some(url) = &server.url {
                    let mut remote = json!({
                        "type": "remote",
                        "url": url,
                        "enabled": true,
                    });
                    if let Some(token) = super::mcp_access_token(&server.id) {
                        remote["headers"] = json!({ "Authorization": format!("Bearer {token}") });
                    }
                    mcp.insert(name, remote);
                }
            }
            McpTransport::Stdio => {
                if let Some(bin) = &server.command {
                    let mut command = vec![bin.clone()];
                    command.extend(server.args.iter().cloned());
                    mcp.insert(
                        name,
                        json!({
                            "type": "local",
                            "command": command,
                            "enabled": true,
                        }),
                    );
                }
            }
        }
    }
}

pub fn merge_grok_catalog(mcp_servers: &mut toml::Table, servers: &[McpServer]) {
    for server in servers {
        let name = sanitize_id(&server.id);
        let mut table = toml::Table::new();
        match server.transport {
            McpTransport::Http => {
                if let Some(url) = &server.url {
                    table.insert("url".into(), toml::Value::String(url.clone()));
                    if let Some(token) = super::mcp_access_token(&server.id) {
                        let mut headers = toml::Table::new();
                        headers.insert(
                            "Authorization".into(),
                            toml::Value::String(format!("Bearer {token}")),
                        );
                        table.insert("http_headers".into(), toml::Value::Table(headers));
                    }
                }
            }
            McpTransport::Stdio => {
                if let Some(bin) = &server.command {
                    table.insert("command".into(), toml::Value::String(bin.clone()));
                }
            }
        }
        if !table.is_empty() {
            mcp_servers.insert(name, toml::Value::Table(table));
        }
    }
}

pub fn merge_claude_catalog(
    mcp_servers: &mut serde_json::Map<String, Value>,
    servers: &[McpServer],
) {
    for server in servers {
        match server.transport {
            McpTransport::Http => {
                if let Some(url) = &server.url {
                    let mut http = json!({
                        "type": "http",
                        "url": url
                    });
                    if let Some(token) = super::mcp_access_token(&server.id) {
                        http["headers"] = json!({ "Authorization": format!("Bearer {token}") });
                    }
                    mcp_servers.insert(server.id.clone(), http);
                }
            }
            McpTransport::Stdio => {
                if let Some(bin) = &server.command {
                    mcp_servers.insert(
                        server.id.clone(),
                        json!({
                            "command": bin,
                            "args": server.args
                        }),
                    );
                }
            }
        }
    }
}

pub fn opencode_catalog_environment(servers: &[McpServer]) -> Vec<(String, String)> {
    if servers.is_empty() {
        return Vec::new();
    }
    let mut mcp = serde_json::Map::new();
    merge_opencode_catalog(&mut mcp, servers);
    let config = json!({ "mcp": mcp });
    serde_json::to_string(&config)
        .ok()
        .map(|content| vec![("OPENCODE_CONFIG_CONTENT".into(), content)])
        .unwrap_or_default()
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn strip_yaml_frontmatter(body: &str) -> String {
    let Some(rest) = body.strip_prefix("---") else {
        return body.trim().to_string();
    };
    let Some(end) = rest.find("\n---") else {
        return body.trim().to_string();
    };
    rest[end + 4..].trim().to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_inject_once() {
        let servers = vec![McpServer {
            id: "cloudflare-api".into(),
            name: "Cloudflare".into(),
            transport: McpTransport::Http,
            url: Some("https://mcp.cloudflare.com/mcp".into()),
            command: None,
            args: Vec::new(),
            enabled: true,
            builtin: true,
            source: Some("catalog".into()),
            ..Default::default()
        }];
        let once = enrich_prompt("hello".into(), &servers);
        assert_eq!(once.matches(CLOUDFLARE_MARKER).count(), 1);
        assert_eq!(enrich_prompt(once.clone(), &servers), once);
    }
}
