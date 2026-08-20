//! Device-local Web3 ship lane: networks, wallets, deploy, and MCP injection.

mod balances;
mod deploy;
mod inject;
mod local_wallet;
mod mcp;
mod networks;
mod okx;
mod toolchain;
mod wallets;

use std::path::{Path, PathBuf};

pub use balances::fetch as fetch_wallet_balances;
pub use deploy::{
    DeployStore, deploy, encode_ctor_args, json_rpc, parse_output_set_digest, preflight,
    scan_artifacts, send_tx,
};
pub use inject::enrich_prompt;
pub use local_wallet::WalletSecrets;
pub use mcp::{
    Web3McpAttachment, attach_claude_mcp, attach_codex_mcp, detect_attachment, merge_grok_mcp,
    merge_opencode_mcp,
};
pub use networks::NetworkStore;
pub use okx::{ONCHAINOS_MCP_NAME, ONCHAINOS_MCP_URL, OkxStore, init as init_okx};
pub use toolchain::{
    cli_env as pf_cli_env, init as init_toolchain, managed_cli, resolve_cli as pf_cli,
    start_install as pf_install, status as pf_status, uninstall as pf_uninstall,
};
pub use wallets::WalletStore;

pub struct Web3Stores {
    pub networks: NetworkStore,
    pub wallets: WalletStore,
    pub okx: OkxStore,
    pub deployments: DeployStore,
}

impl Web3Stores {
    pub fn new(data_dir: &Path) -> Self {
        let root = web3_dir(data_dir);
        init_okx(&root);
        init_toolchain(data_dir);
        Self {
            networks: NetworkStore::new(&root),
            wallets: WalletStore::new(&root),
            okx: OkxStore::new(&root),
            deployments: DeployStore::new(&root),
        }
    }
}

pub fn web3_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("web3")
}
