use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How an MCP server is reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum McpTransport {
    Http,
    Stdio,
}

impl Default for McpTransport {
    fn default() -> Self {
        Self::Http
    }
}

/// One row in Settings → MCP.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub builtin: bool,
    /// `catalog`, `web3-okx`, or `web3-pf`. Read-only rows are not user-owned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `none` | `public` | `needed` | `authorized`. Computed by the daemon.
    #[serde(default)]
    pub auth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_account: Option<String>,
}

/// What lives under a session cwd that we can preview or host.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDetect {
    /// `none` | `static` | `vite` | `next` | `worker`
    pub kind: String,
    pub root: String,
    pub hint: String,
    #[serde(default)]
    pub spa: bool,
    #[serde(default)]
    pub wrangler: bool,
    #[serde(default)]
    pub vercel: bool,
    pub project_name: String,
}

/// Live local preview for one cwd.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewStatus {
    pub detect: FrontendDetect,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Hosting target for a one-click frontend deploy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HostingProvider {
    Cloudflare,
    Vercel,
}

impl HostingProvider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::Vercel => "vercel",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "cloudflare" => Some(Self::Cloudflare),
            "vercel" => Some(Self::Vercel),
            _ => None,
        }
    }
}

/// CLI token status. The raw token never leaves the daemon.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostingTokenStatus {
    pub provider: String,
    pub configured: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hint: Option<String>,
}

/// One hosted frontend deploy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostingRecord {
    pub id: String,
    pub provider: String,
    pub kind: String,
    pub url: String,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub ts: String,
}

/// Mixed ship history row for one cwd.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipHistoryItem {
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub fn builtin_mcp_servers() -> Vec<McpServer> {
    vec![
        McpServer {
            id: "github".into(),
            name: "GitHub".into(),
            transport: McpTransport::Http,
            url: Some("https://api.githubcopilot.com/mcp/".into()),
            command: None,
            args: Vec::new(),
            enabled: false,
            builtin: true,
            source: Some("catalog".into()),
            auth: String::new(),
            auth_account: None,
        },
        McpServer {
            id: "cloudflare-docs".into(),
            name: "Cloudflare Docs".into(),
            transport: McpTransport::Http,
            url: Some("https://docs.mcp.cloudflare.com/mcp".into()),
            command: None,
            args: Vec::new(),
            enabled: false,
            builtin: true,
            source: Some("catalog".into()),
            auth: String::new(),
            auth_account: None,
        },
        McpServer {
            id: "cloudflare-api".into(),
            name: "Cloudflare".into(),
            transport: McpTransport::Http,
            url: Some("https://mcp.cloudflare.com/mcp".into()),
            command: None,
            args: Vec::new(),
            enabled: false,
            builtin: true,
            source: Some("catalog".into()),
            auth: String::new(),
            auth_account: None,
        },
        McpServer {
            id: "cloudflare-bindings".into(),
            name: "Cloudflare Bindings".into(),
            transport: McpTransport::Http,
            url: Some("https://bindings.mcp.cloudflare.com/mcp".into()),
            command: None,
            args: Vec::new(),
            enabled: false,
            builtin: true,
            source: Some("catalog".into()),
            auth: String::new(),
            auth_account: None,
        },
        McpServer {
            id: "vercel".into(),
            name: "Vercel".into(),
            transport: McpTransport::Http,
            url: Some("https://mcp.vercel.com".into()),
            command: None,
            args: Vec::new(),
            enabled: false,
            builtin: true,
            source: Some("catalog".into()),
            auth: String::new(),
            auth_account: None,
        },
    ]
}

pub fn is_builtin_mcp_id(id: &str) -> bool {
    builtin_mcp_servers().iter().any(|server| server.id == id)
}
