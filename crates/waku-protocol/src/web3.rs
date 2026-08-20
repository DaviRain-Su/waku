use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One EVM network the operator can deploy to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EvmNetwork {
    pub id: String,
    pub name: String,
    pub chain_id: u64,
    pub rpc_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    pub currency_symbol: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// How an address-book row was added.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum WalletSource {
    /// Leftover from a removed WalletConnect bridge. Cannot sign or be created.
    WalletConnect,
    /// Watch-only address. Cannot sign.
    Watch,
    /// Env var *name* holding a hex key. Testnet-only.
    DevEnvKey,
    /// In-app signer. The private key lives under the daemon data dir.
    Local,
}

/// One signer or watch address the operator can pick at deploy time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub address: String,
    pub source: WalletSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_key_name: Option<String>,
}

/// Native-token balance on one network. `wei` is a decimal integer string so
/// JS never sees a 256-bit quantity as a number. `display` is already trimmed
/// for the UI (up to 6 fractional digits).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalance {
    pub network_id: String,
    pub network_name: String,
    pub symbol: String,
    pub wei: String,
    pub display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Live balances for one address-book row. Not persisted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalanceSnapshot {
    pub wallet_id: String,
    pub address: String,
    pub balances: Vec<WalletBalance>,
}

/// One-time create response. `backup_hex` is never persisted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWallet {
    pub wallets: Vec<WalletAccount>,
    pub wallet: WalletAccount,
    pub backup_hex: String,
}

/// OnchainOS status. The raw key never leaves the daemon.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OkxStatus {
    pub configured: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hint: Option<String>,
}

/// A gate-passing artifact set found under a session working tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeployArtifact {
    pub module: String,
    pub dir: String,
    pub bin_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default)]
    pub modified_ms: u64,
}

/// One on-chain deployment of a gate-passing artifact set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRecord {
    pub id: String,
    pub module: String,
    pub network_id: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    pub tx_hash: String,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// ProofForge compiler install as seen by Settings.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PfToolchainStatus {
    /// `missing` | `installing` | `ready`
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    /// `managed` (Waku download) or `host` (PATH / env).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub evm_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Built-in EVM presets. X Layer first.
pub fn builtin_networks() -> Vec<EvmNetwork> {
    vec![
        EvmNetwork {
            id: "xlayer-testnet".into(),
            name: "X Layer Testnet".into(),
            chain_id: 1952,
            rpc_url: "https://testrpc.xlayer.tech/terigon".into(),
            explorer_url: Some("https://www.oklink.com/xlayer-test".into()),
            currency_symbol: "OKB".into(),
            builtin: true,
            enabled: true,
        },
        EvmNetwork {
            id: "xlayer-mainnet".into(),
            name: "X Layer".into(),
            chain_id: 196,
            rpc_url: "https://rpc.xlayer.tech".into(),
            explorer_url: Some("https://www.okx.com/web3/explorer/xlayer".into()),
            currency_symbol: "OKB".into(),
            builtin: true,
            enabled: true,
        },
        EvmNetwork {
            id: "ethereum-sepolia".into(),
            name: "Ethereum Sepolia".into(),
            chain_id: 11155111,
            rpc_url: "https://ethereum-sepolia-rpc.publicnode.com".into(),
            explorer_url: Some("https://sepolia.etherscan.io".into()),
            currency_symbol: "ETH".into(),
            builtin: true,
            enabled: true,
        },
        EvmNetwork {
            id: "base-sepolia".into(),
            name: "Base Sepolia".into(),
            chain_id: 84532,
            rpc_url: "https://sepolia.base.org".into(),
            explorer_url: Some("https://sepolia.basescan.org".into()),
            currency_symbol: "ETH".into(),
            builtin: true,
            enabled: true,
        },
        EvmNetwork {
            id: "anvil".into(),
            name: "Anvil".into(),
            chain_id: 31337,
            rpc_url: "http://127.0.0.1:8545".into(),
            explorer_url: None,
            currency_symbol: "ETH".into(),
            builtin: true,
            enabled: false,
        },
    ]
}

pub fn default_network_id() -> &'static str {
    "xlayer-testnet"
}

pub fn is_builtin_network_id(id: &str) -> bool {
    builtin_networks().iter().any(|network| network.id == id)
}

/// Extra public endpoints to try when `rpc_url` fails. Order is primary first.
pub fn rpc_urls(network: &EvmNetwork) -> Vec<String> {
    let mut urls = vec![network.rpc_url.trim().to_string()];
    let extras: &[&str] = match network.id.as_str() {
        "xlayer-testnet" => &[
            "https://testrpc.xlayer.tech/terigon",
            "https://xlayertestrpc.okx.com/terigon",
            "https://testrpc.xlayer.tech",
            "https://xlayertestrpc.okx.com",
        ],
        "xlayer-mainnet" => &["https://rpc.xlayer.tech", "https://xlayerrpc.okx.com"],
        _ => &[],
    };
    for extra in extras {
        if !urls.iter().any(|url| url == extra) {
            urls.push((*extra).to_string());
        }
    }
    urls.retain(|url| !url.is_empty());
    urls
}

/// X Layer testnet faucet. Other networks, including Anvil, have none.
pub fn faucet_url(network_id: &str) -> Option<&'static str> {
    (network_id == "xlayer-testnet").then_some("https://web3.okx.com/xlayer/faucet")
}

pub fn explorer_address_url(explorer_url: &str, address: &str) -> String {
    format!(
        "{}/address/{}",
        explorer_url.trim_end_matches('/'),
        address
    )
}

/// Format a decimal wei amount as a display string with up to 6 fractional
/// digits. Values below 10⁻⁶ still show as `0.000000` so a dust balance is
/// not mistaken for empty.
pub fn format_wei(wei: &str) -> String {
    let digits: String = wei.trim().chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.chars().all(|c| c == '0') {
        return "0".into();
    }
    let padded = format!("{digits:0>18}");
    let split = padded.len() - 18;
    let whole = match padded[..split].trim_start_matches('0') {
        "" => "0",
        rest => rest,
    };
    let frac = &padded[split..];
    let frac6 = &frac[..6.min(frac.len())];
    let frac_trimmed = frac6.trim_end_matches('0');
    if !frac_trimmed.is_empty() {
        format!("{whole}.{frac_trimmed}")
    } else if frac.chars().any(|c| c != '0') {
        format!("{whole}.000000")
    } else {
        whole.to_string()
    }
}

pub fn short_digest(digest: &str) -> String {
    let short: String = digest.chars().take(10).collect();
    if digest.chars().count() > 10 {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anvil_preset_is_local_and_off() {
        let anvil = builtin_networks()
            .into_iter()
            .find(|network| network.id == "anvil")
            .expect("anvil preset");
        assert_eq!(anvil.chain_id, 31337);
        assert_eq!(anvil.rpc_url, "http://127.0.0.1:8545");
        assert!(anvil.explorer_url.is_none());
        assert!(anvil.builtin);
        assert!(!anvil.enabled);
        assert!(is_builtin_network_id("anvil"));
    }

    #[test]
    fn faucet_is_xlayer_testnet_only() {
        assert_eq!(
            faucet_url("xlayer-testnet"),
            Some("https://web3.okx.com/xlayer/faucet")
        );
        assert_eq!(faucet_url("anvil"), None);
        assert_eq!(faucet_url("xlayer-mainnet"), None);
    }

    #[test]
    fn format_wei_trims_trailing_zeros() {
        assert_eq!(format_wei("0"), "0");
        assert_eq!(format_wei("1000000000000000000"), "1");
        assert_eq!(format_wei("1500000000000000000"), "1.5");
        assert_eq!(format_wei("123456000000000000"), "0.123456");
        assert_eq!(format_wei("1"), "0.000000");
    }

    #[test]
    fn explorer_and_digest_helpers() {
        assert_eq!(
            explorer_address_url("https://sepolia.etherscan.io/", "0xabc"),
            "https://sepolia.etherscan.io/address/0xabc"
        );
        assert_eq!(short_digest("abcdefghij"), "abcdefghij");
        assert_eq!(short_digest("abcdefghijkl"), "abcdefghij…");
    }
}
