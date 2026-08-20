//! MCP catalog, local preview, and hosting deploys.

mod hosting;
mod inject;
mod mcp;
mod oauth;
mod preview;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use waku_protocol::ship::{McpServer, ShipHistoryItem};
use waku_protocol::web3::{explorer_address_url, DeploymentRecord, EvmNetwork};

pub use hosting::HostingStore;
pub use inject::{
    attach_codex_catalog, catalog_servers, enrich_prompt, merge_claude_catalog, merge_grok_catalog,
    merge_opencode_catalog, opencode_catalog_environment,
};
pub use mcp::McpCatalog;
pub use oauth::{OAuthStore, access_token as mcp_access_token};
pub use preview::{detect_frontend, scan as scan_frontend, start as start_preview, status as preview_status, stop as stop_preview};

pub struct ShipStores {
    pub mcp: McpCatalog,
    pub hosting: HostingStore,
    pub oauth: OAuthStore,
}

static CATALOG: OnceLock<McpCatalog> = OnceLock::new();

impl ShipStores {
    pub fn new(data_dir: &Path) -> Self {
        let stores = Self::open(data_dir);
        let _ = CATALOG.set(stores.mcp.clone());
        stores
    }

    fn open(data_dir: &Path) -> Self {
        let root = data_dir.join("ship");
        Self {
            mcp: McpCatalog::new(&root),
            hosting: HostingStore::new(&root),
            oauth: OAuthStore::new(&root),
        }
    }

    pub fn history(
        &self,
        cwd: Option<&Path>,
        contracts: &[DeploymentRecord],
        networks: &[EvmNetwork],
    ) -> Vec<ShipHistoryItem> {
        let cwd_text = cwd.map(|path| path.display().to_string());
        let mut items = Vec::new();
        for record in contracts {
            if cwd_text
                .as_deref()
                .is_some_and(|cwd| record.cwd.as_deref().is_some_and(|row| row != cwd))
            {
                continue;
            }
            items.push(ShipHistoryItem {
                kind: "contract".into(),
                title: record.module.clone(),
                detail: record.address.clone(),
                ts: record.ts.clone(),
                url: networks
                    .iter()
                    .find(|network| network.id == record.network_id)
                    .and_then(|network| network.explorer_url.as_deref())
                    .map(|base| explorer_address_url(base, &record.address)),
            });
        }
        if let Ok(hosting) = self.hosting.history() {
            for record in hosting {
                if cwd_text
                    .as_deref()
                    .is_some_and(|cwd| record.cwd.as_deref().is_some_and(|row| row != cwd))
                {
                    continue;
                }
                items.push(ShipHistoryItem {
                    kind: "hosting".into(),
                    title: format!("{} · {}", record.provider, record.project_name),
                    detail: record.url.clone(),
                    ts: record.ts.clone(),
                    url: Some(record.url),
                });
            }
        }
        items.sort_by(|left, right| right.ts.cmp(&left.ts));
        items
    }
}

pub fn enabled_catalog() -> Vec<McpServer> {
    CATALOG
        .get()
        .map(McpCatalog::enabled)
        .unwrap_or_default()
}

pub fn ship_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("ship")
}

#[cfg(test)]
mod tests {
    use super::*;
    use waku_protocol::web3::DeploymentRecord;

    #[test]
    fn history_merges_contract_and_hosting_by_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let stores = ShipStores::open(dir.path());
        stores
            .hosting
            .append(waku_protocol::ship::HostingRecord {
                id: "h1".into(),
                provider: "cloudflare".into(),
                kind: "pages".into(),
                url: "https://app.pages.dev".into(),
                project_name: "app".into(),
                cwd: Some("/tmp/app".into()),
                ts: "2026-01-02T00:00:00Z".into(),
            })
            .unwrap();
        let contracts = vec![DeploymentRecord {
            id: "c1".into(),
            module: "Share".into(),
            network_id: "ethereum-sepolia".into(),
            address: "0xabc".into(),
            ctor: None,
            digest: None,
            tx_hash: "0xdef".into(),
            ts: "2026-01-01T00:00:00Z".into(),
            cwd: Some("/tmp/app".into()),
        }];
        let sepolia = waku_protocol::web3::builtin_networks()
            .into_iter()
            .find(|network| network.id == "ethereum-sepolia")
            .unwrap();
        let items = stores.history(
            Some(std::path::Path::new("/tmp/app")),
            &contracts,
            &[sepolia],
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "hosting");
        assert_eq!(items[1].kind, "contract");
        assert_eq!(
            items[1].url.as_deref(),
            Some("https://sepolia.etherscan.io/address/0xabc")
        );
        let other = stores.history(
            Some(std::path::Path::new("/tmp/other")),
            &contracts,
            &[],
        );
        assert!(other.is_empty());
    }
}
