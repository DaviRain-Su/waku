//! One-click deploy of sealed ProofForge gate artifacts.

use std::path::{Path, PathBuf};

use alloy::dyn_abi::{DynSolType, DynSolValue};
use anyhow::{anyhow, bail};
use chrono::Utc;
use uuid::Uuid;
use waku_protocol::web3::{
    DeployArtifact, DeploymentRecord, EvmNetwork, WalletAccount, WalletSource,
};

use super::local_wallet::{send_with_key, send_with_local, WalletSecrets};

const MAX_DEPLOYMENTS: usize = 100;
const SCAN_MAX_DEPTH: usize = 4;
const SCAN_MAX_DIRS: usize = 2000;
const MAX_BIN_BYTES: u64 = 4 * 1024 * 1024;
const MAINNET_CHAIN_IDS: &[u64] = &[
    1, 10, 56, 137, 196, 8453, 42161, 43114,
];

pub struct DeployStore {
    file: PathBuf,
}

impl DeployStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("deployments.json"),
        }
    }

    pub fn load(&self) -> anyhow::Result<Vec<DeploymentRecord>> {
        match std::fs::read_to_string(&self.file) {
            Ok(raw) => {
                let mut deployments: Vec<DeploymentRecord> = serde_json::from_str(&raw)?;
                deployments.truncate(MAX_DEPLOYMENTS);
                Ok(deployments)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn append(&self, record: DeploymentRecord) -> anyhow::Result<Vec<DeploymentRecord>> {
        let mut deployments = self.load()?;
        deployments.insert(0, record);
        deployments.truncate(MAX_DEPLOYMENTS);
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&deployments)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(deployments)
    }
}

pub fn scan_artifacts(cwd: &Path) -> Vec<DeployArtifact> {
    let mut artifacts = Vec::new();
    let mut stack = vec![(cwd.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        visited += 1;
        if visited > SCAN_MAX_DIRS {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth < SCAN_MAX_DEPTH && !skip_dir(&name) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !meta.is_file() || path.extension().is_none_or(|ext| ext != "bin") {
                continue;
            }
            let Some(module) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|module| valid_module(module))
            else {
                continue;
            };
            if meta.len() == 0 || meta.len() > MAX_BIN_BYTES || !is_hex_bytecode(&path) {
                continue;
            }
            let abi = path.with_file_name(format!("{module}.abi.json"));
            artifacts.push(DeployArtifact {
                module,
                dir: relative_dir(cwd, &dir),
                bin_path: path.to_string_lossy().into_owned(),
                abi_path: abi.is_file().then(|| abi.to_string_lossy().into_owned()),
                digest: digest_near(&dir),
                modified_ms: modified_ms(&meta),
            });
        }
    }
    artifacts.sort_by(|left, right| right.modified_ms.cmp(&left.modified_ms));
    artifacts
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "dist" | "build" | "vendor" | "__pycache__"
        )
}

fn valid_module(module: &str) -> bool {
    let mut chars = module.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && module.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_hex_bytecode(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 256];
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    let text = String::from_utf8_lossy(&head[..n]);
    let text = text.trim_start().trim_start_matches("0x");
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c.is_ascii_whitespace())
}

fn digest_near(dir: &Path) -> Option<String> {
    for name in ["gate-report.json", "inspect.json", "inspect.txt"] {
        let path = dir.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(digest) = parse_output_set_digest(&raw) {
            return Some(digest);
        }
    }
    None
}

pub fn parse_output_set_digest(raw: &str) -> Option<String> {
    let idx = raw.find("outputSetDigest")?;
    raw[idx..]
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|part| part.len() == 64 && part.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
}

fn relative_dir(cwd: &Path, dir: &Path) -> String {
    dir.strip_prefix(cwd)
        .map(|path| {
            let text = path.to_string_lossy();
            if text.is_empty() {
                ".".into()
            } else {
                text.into_owned()
            }
        })
        .unwrap_or_else(|_| dir.to_string_lossy().into_owned())
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn encode_ctor_args(sig: &str, args: &[String]) -> anyhow::Result<Vec<u8>> {
    let inner = sig.trim();
    let inner = if inner == "-" { "" } else { inner };
    let inner = inner.strip_prefix("constructor").unwrap_or(inner).trim();
    let inner = inner
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(inner)
        .trim();
    if inner.is_empty() {
        return if args.is_empty() {
            Ok(Vec::new())
        } else {
            Err(anyhow!(
                "constructor signature lists no parameters but {} args were given",
                args.len()
            ))
        };
    }
    let tuple: DynSolType = format!("({inner})")
        .parse()
        .map_err(|error| anyhow!("bad constructor signature '{sig}': {error}"))?;
    let DynSolType::Tuple(types) = tuple else {
        bail!("bad constructor signature '{sig}'");
    };
    if types.len() != args.len() {
        bail!(
            "constructor expects {} args, got {}",
            types.len(),
            args.len()
        );
    }
    let mut values = Vec::with_capacity(types.len());
    for (ty, arg) in types.iter().zip(args) {
        let value = ty
            .coerce_str(arg.trim())
            .map_err(|error| anyhow!("arg '{arg}' does not fit {ty}: {error}"))?;
        values.push(value);
    }
    Ok(DynSolValue::Tuple(values).abi_encode_params())
}

pub fn preflight(network: &EvmNetwork, wallet: &WalletAccount) -> anyhow::Result<()> {
    if !network.enabled {
        bail!(
            "network {} is disabled — enable it in Settings → Networks first",
            network.name
        );
    }
    match wallet.source {
        WalletSource::Watch => bail!("watch-only wallets cannot sign deploy transactions"),
        WalletSource::WalletConnect => {
            bail!("WalletConnect was removed — create or import a local key in Settings → Wallets")
        }
        WalletSource::DevEnvKey => {
            if MAINNET_CHAIN_IDS.contains(&network.chain_id) {
                bail!(
                    "DevEnvKey wallets cannot deploy to mainnet (chain id {}); use a testnet only",
                    network.chain_id
                );
            }
            if wallet
                .env_key_name
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                bail!("DevEnvKey wallet missing env_key_name");
            }
        }
        WalletSource::Local => {
            if wallet.address.trim().is_empty() {
                bail!("local wallet has no address — create or import it again");
            }
        }
    }
    Ok(())
}

pub fn deploy(
    bin_path: &str,
    module: &str,
    ctor_sig: &str,
    ctor_args: &[String],
    digest: Option<String>,
    cwd: Option<String>,
    network: &EvmNetwork,
    wallet: &WalletAccount,
    secrets: &WalletSecrets,
    store: &DeployStore,
) -> anyhow::Result<DeploymentRecord> {
    preflight(network, wallet)?;
    let bytecode = read_bytecode(Path::new(bin_path))?;
    let ctor_hex = if ctor_sig.trim().is_empty() || ctor_sig.trim() == "-" {
        String::new()
    } else {
        alloy::hex::encode(encode_ctor_args(ctor_sig, ctor_args)?)
    };
    let create_data = format!("0x{bytecode}{ctor_hex}");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow!("could not start deploy runtime: {error}"))?;
    let (address, tx_hash) = runtime.block_on(async {
        match wallet.source {
            WalletSource::Watch => bail!("watch-only wallets cannot sign"),
            WalletSource::Local => {
                let sent = send_with_local(
                    secrets,
                    &wallet.id,
                    &network.rpc_url,
                    network.chain_id,
                    None,
                    &create_data,
                )
                .await?;
                match sent.contract_address {
                    Some(address) => Ok((address, sent.tx_hash)),
                    None => bail!("deploy tx {} mined without a contract address", sent.tx_hash),
                }
            }
            WalletSource::DevEnvKey => {
                let env_name = wallet.env_key_name.as_deref().unwrap_or("");
                let key = match std::env::var(env_name) {
                    Ok(key) if !key.trim().is_empty() => key,
                    Ok(_) => bail!("env var '{env_name}' is empty"),
                    Err(_) => bail!("env var '{env_name}' is not set"),
                };
                let sent = send_with_key(
                    key.trim(),
                    &network.rpc_url,
                    network.chain_id,
                    None,
                    &create_data,
                )
                .await?;
                match sent.contract_address {
                    Some(address) => Ok((address, sent.tx_hash)),
                    None => bail!("deploy tx {} mined without a contract address", sent.tx_hash),
                }
            }
            WalletSource::WalletConnect => {
                bail!("WalletConnect was removed — create or import a local key")
            }
        }
    })?;

    let record = DeploymentRecord {
        id: Uuid::new_v4().to_string(),
        module: module.to_string(),
        network_id: network.id.clone(),
        address,
        ctor: Some(ctor_sig.trim())
            .filter(|sig| !sig.is_empty() && *sig != "-")
            .map(str::to_string),
        digest,
        tx_hash,
        ts: Utc::now().to_rfc3339(),
        cwd,
    };
    store
        .append(record.clone())
        .map_err(|error| anyhow!("deployed on-chain but failed to persist record: {error}"))?;
    Ok(record)
}

/// Sign and broadcast a dapp `eth_sendTransaction` with the in-app signer.
/// `to` is `None` for contract creation. Alloy fills nonce, gas, and fees.
pub fn send_tx(
    to: Option<&str>,
    data: &str,
    network: &EvmNetwork,
    wallet: &WalletAccount,
    secrets: &WalletSecrets,
) -> anyhow::Result<String> {
    preflight(network, wallet)?;
    let data = data.trim();
    let data = if data.is_empty() { "0x" } else { data };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow!("could not start send runtime: {error}"))?;
    runtime.block_on(async {
        match wallet.source {
            WalletSource::Watch => bail!("watch-only wallets cannot sign"),
            WalletSource::Local => {
                let sent = send_with_local(
                    secrets,
                    &wallet.id,
                    &network.rpc_url,
                    network.chain_id,
                    to,
                    data,
                )
                .await?;
                Ok(sent.tx_hash)
            }
            WalletSource::DevEnvKey => {
                let env_name = wallet.env_key_name.as_deref().unwrap_or("");
                let key = match std::env::var(env_name) {
                    Ok(key) if !key.trim().is_empty() => key,
                    Ok(_) => bail!("env var '{env_name}' is empty"),
                    Err(_) => bail!("env var '{env_name}' is not set"),
                };
                let sent = send_with_key(
                    key.trim(),
                    &network.rpc_url,
                    network.chain_id,
                    to,
                    data,
                )
                .await?;
                Ok(sent.tx_hash)
            }
            WalletSource::WalletConnect => {
                bail!("WalletConnect was removed — create or import a local key")
            }
        }
    })
}

pub fn json_rpc(
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut urls = vec![rpc_url.trim().to_string()];
    if rpc_url.contains("xlayer") {
        for extra in [
            "https://testrpc.xlayer.tech/terigon",
            "https://xlayertestrpc.okx.com/terigon",
            "https://testrpc.xlayer.tech",
            "https://xlayertestrpc.okx.com",
        ] {
            if !urls.iter().any(|url| url == extra) {
                urls.push(extra.to_string());
            }
        }
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;
    let mut last_error = anyhow!("no RPC URL");
    for url in urls {
        match json_rpc_once(&client, &url, method, &params) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn json_rpc_once(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    method: &str,
    params: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let response = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|error| anyhow!("RPC {method} via {rpc_url}: {error}"))?;
    let body: serde_json::Value = response
        .json()
        .map_err(|error| anyhow!("RPC {method} decode via {rpc_url}: {error}"))?;
    if let Some(error) = body.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("RPC error");
        bail!("{message}");
    }
    Ok(body
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

fn read_bytecode(path: &Path) -> anyhow::Result<String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("bin missing or unreadable at {}: {error}", path.display()))?;
    let stripped: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let hex = stripped.trim_start_matches("0x");
    if hex.is_empty() {
        bail!("bin file is empty: {}", path.display());
    }
    if hex.len() % 2 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("bin file is not hex bytecode: {}", path.display());
    }
    Ok(hex.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use waku_protocol::web3::builtin_networks;

    fn xlayer_testnet() -> EvmNetwork {
        builtin_networks()
            .into_iter()
            .find(|network| network.id == "xlayer-testnet")
            .unwrap()
    }

    fn xlayer_mainnet() -> EvmNetwork {
        builtin_networks()
            .into_iter()
            .find(|network| network.id == "xlayer-mainnet")
            .unwrap()
    }

    #[test]
    fn store_roundtrip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeployStore::new(dir.path());
        let record = |id: &str| DeploymentRecord {
            id: id.into(),
            module: "Mod".into(),
            network_id: "xlayer-testnet".into(),
            address: "0xabc".into(),
            ctor: None,
            digest: None,
            tx_hash: "0xtx".into(),
            ts: "2026-08-13T00:00:00Z".into(),
            cwd: None,
        };
        store.append(record("d1")).unwrap();
        let list = store.append(record("d2")).unwrap();
        assert_eq!(list[0].id, "d2");
        assert_eq!(list[1].id, "d1");
    }

    #[test]
    fn scan_finds_bin_with_abi_and_digest() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out-evm");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("EscrowVault.bin"), "0x6080604052\n").unwrap();
        std::fs::write(out.join("EscrowVault.abi.json"), "[]").unwrap();
        let digest = "a".repeat(64);
        std::fs::write(
            out.join("gate-report.json"),
            format!(r#"{{"outputSetDigest":"{digest}"}}"#),
        )
        .unwrap();
        std::fs::write(out.join("notes.bin"), "not hex at all!").unwrap();
        std::fs::create_dir_all(temp.path().join(".git")).unwrap();
        std::fs::write(temp.path().join(".git/junk.bin"), "6080").unwrap();

        let artifacts = scan_artifacts(temp.path());
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert_eq!(artifacts[0].module, "EscrowVault");
        assert_eq!(artifacts[0].dir, "out-evm");
        assert!(artifacts[0].abi_path.is_some());
        assert_eq!(artifacts[0].digest.as_deref(), Some(digest.as_str()));
    }

    #[test]
    fn ctor_encoding_matches_abi() {
        assert!(encode_ctor_args("", &[]).unwrap().is_empty());
        let encoded =
            encode_ctor_args("constructor(uint64,uint64)", &["7".into(), "9".into()]).unwrap();
        assert_eq!(encoded.len(), 64);
        assert_eq!(encoded[31], 7);
        assert_eq!(encoded[63], 9);
        assert!(encode_ctor_args("(uint64)", &[]).is_err());
        assert!(encode_ctor_args("", &["7".into()]).is_err());
    }

    #[test]
    fn preflight_policy() {
        let wallet = |source: WalletSource| WalletAccount {
            id: "w".into(),
            label: "w".into(),
            address: "0x0000000000000000000000000000000000000001".into(),
            source,
            env_key_name: Some("PF_XLAYER_KEY".into()),
        };
        assert!(preflight(&xlayer_testnet(), &wallet(WalletSource::Watch)).is_err());
        preflight(&xlayer_testnet(), &wallet(WalletSource::Local)).unwrap();
        preflight(&xlayer_mainnet(), &wallet(WalletSource::Local)).unwrap();
        preflight(&xlayer_testnet(), &wallet(WalletSource::DevEnvKey)).unwrap();
        assert!(preflight(&xlayer_mainnet(), &wallet(WalletSource::DevEnvKey)).is_err());
        let mut disabled = xlayer_testnet();
        disabled.enabled = false;
        assert!(preflight(&disabled, &wallet(WalletSource::Local)).is_err());
    }
}
