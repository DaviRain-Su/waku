//! In-app Alloy signer. Metadata lives in `wallets.json`; hex keys live under
//! `web3/wallet-secrets/{id}` with mode 0600.

use std::path::{Path, PathBuf};

use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{Address, Bytes};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context as _, anyhow, bail};

pub struct WalletSecrets {
    dir: PathBuf,
}

impl WalletSecrets {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("wallet-secrets"),
        }
    }

    pub fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    pub fn put(&self, id: &str, hex_key: &str) -> anyhow::Result<()> {
        if id.trim().is_empty() || id.contains('/') || id.contains('\\') {
            bail!("invalid wallet id");
        }
        let hex_key = normalize_hex_key(hex_key)?;
        std::fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&self.dir)
                .map(|meta| meta.permissions())
                .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o700));
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(&self.dir, perms);
        }
        let path = self.path_for(id);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, hex_key.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> anyhow::Result<String> {
        let raw = std::fs::read_to_string(self.path_for(id)).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                anyhow!("local wallet {id} has no stored key — recreate or import it")
            } else {
                err.into()
            }
        })?;
        normalize_hex_key(raw.trim())
    }

    pub fn delete(&self, id: &str) {
        let _ = std::fs::remove_file(self.path_for(id));
    }
}

pub fn generate_local_key() -> anyhow::Result<(String, String)> {
    let signer = PrivateKeySigner::random();
    let backup = format!("0x{}", alloy::hex::encode(signer.to_bytes()));
    Ok((backup, format_address(signer.address())))
}

pub fn import_local_key(secret: &str) -> anyhow::Result<(String, String)> {
    let signer = parse_signer(secret)?;
    let backup = format!("0x{}", alloy::hex::encode(signer.to_bytes()));
    Ok((backup, format_address(signer.address())))
}

pub fn parse_signer(secret: &str) -> anyhow::Result<PrivateKeySigner> {
    let secret = secret.trim();
    if secret.is_empty() {
        bail!("secret is empty");
    }
    secret
        .parse::<PrivateKeySigner>()
        .with_context(|| "could not parse private key (hex 0x…64)")
}

pub struct SendOutcome {
    pub tx_hash: String,
    pub contract_address: Option<String>,
}

pub async fn send_with_local(
    secrets: &WalletSecrets,
    wallet_id: &str,
    rpc_url: &str,
    chain_id: u64,
    to: Option<&str>,
    data: &str,
) -> anyhow::Result<SendOutcome> {
    let hex_key = secrets.get(wallet_id)?;
    send_with_key(&hex_key, rpc_url, chain_id, to, data).await
}

pub async fn send_with_key(
    hex_key: &str,
    rpc_url: &str,
    chain_id: u64,
    to: Option<&str>,
    data: &str,
) -> anyhow::Result<SendOutcome> {
    let signer = parse_signer(hex_key)?;
    let from = signer.address();
    let wallet = EthereumWallet::from(signer);
    let urls = rpc_fallback_urls(rpc_url);
    let mut last_error = anyhow!("no RPC URL");
    for url in urls {
        match send_with_key_on_url(&wallet, from, &url, chain_id, to, data).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn rpc_fallback_urls(rpc_url: &str) -> Vec<String> {
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
    urls.retain(|url| !url.is_empty());
    urls
}

async fn send_with_key_on_url(
    wallet: &EthereumWallet,
    from: Address,
    rpc_url: &str,
    chain_id: u64,
    to: Option<&str>,
    data: &str,
) -> anyhow::Result<SendOutcome> {
    let url: reqwest::Url = rpc_url
        .parse()
        .map_err(|error| anyhow!("invalid rpc url {rpc_url}: {error}"))?;
    let provider = ProviderBuilder::new()
        .wallet(wallet.clone())
        .connect_http(url);

    let input: Bytes = data
        .parse()
        .map_err(|error| anyhow!("invalid tx data: {error}"))?;
    let mut tx = TransactionRequest::default()
        .with_chain_id(chain_id)
        .with_from(from)
        .with_input(input);
    if let Some(to) = to {
        let addr: Address = to
            .parse()
            .map_err(|error| anyhow!("invalid to address: {error}"))?;
        tx = tx.with_to(addr);
    }

    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|error| anyhow!("send transaction via {rpc_url}: {error}"))?;
    let tx_hash = format!("{:#x}", pending.tx_hash());
    // Receipt is best-effort. Public X Layer RPCs often drop the follow-up
    // poll after a successful broadcast (`error sending request for url`).
    let contract_address =
        match tokio::time::timeout(std::time::Duration::from_secs(20), pending.get_receipt()).await
        {
            Ok(Ok(receipt)) => receipt
                .contract_address
                .map(|address| format!("{address:#x}")),
            _ => None,
        };
    Ok(SendOutcome {
        tx_hash,
        contract_address,
    })
}

fn format_address(address: Address) -> String {
    format!("{address:#x}")
}

fn normalize_hex_key(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("private key must be 32 bytes hex");
    }
    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_reload_same_address() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = WalletSecrets::new(dir.path());
        let (backup, address) = generate_local_key().unwrap();
        secrets.put("w1", &backup).unwrap();
        let loaded = secrets.get("w1").unwrap();
        let signer = parse_signer(&loaded).unwrap();
        assert_eq!(format_address(signer.address()), address);
        assert!(
            !std::fs::read_to_string(secrets.path_for("w1"))
                .unwrap()
                .contains(&address[2..])
        );
    }

    #[test]
    fn import_hex_roundtrip() {
        let (backup, address) = generate_local_key().unwrap();
        let (again, imported) = import_local_key(&backup).unwrap();
        assert_eq!(imported, address);
        assert_eq!(again, backup);
    }

    #[test]
    fn rejects_empty_and_short_keys() {
        assert!(import_local_key("").is_err());
        assert!(import_local_key("0x1234").is_err());
        assert!(normalize_hex_key("not-a-key").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_owner_rw_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secrets = WalletSecrets::new(dir.path());
        let (backup, _) = generate_local_key().unwrap();
        secrets.put("w1", &backup).unwrap();
        let mode = std::fs::metadata(secrets.path_for("w1"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
