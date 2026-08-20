//! Local wallet address book. Private keys never appear in `wallets.json`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _};
use waku_protocol::web3::{WalletAccount, WalletSource};

use super::local_wallet::{self, WalletSecrets};

pub struct WalletStore {
    file: PathBuf,
    secrets: WalletSecrets,
}

impl WalletStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("wallets.json"),
            secrets: WalletSecrets::new(data_dir),
        }
    }

    pub fn secrets(&self) -> &WalletSecrets {
        &self.secrets
    }

    pub fn create_local(
        &self,
        label: &str,
    ) -> anyhow::Result<(Vec<WalletAccount>, WalletAccount, String)> {
        let label = label.trim();
        if label.is_empty() {
            bail!("label must not be empty");
        }
        let (backup, address) = local_wallet::generate_local_key()?;
        let wallet = WalletAccount {
            id: format!("local-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            label: label.to_string(),
            address,
            source: WalletSource::Local,
            env_key_name: None,
        };
        self.secrets.put(&wallet.id, &backup)?;
        let wallets = self.upsert(wallet.clone())?;
        Ok((wallets, wallet, backup))
    }

    pub fn import_local(
        &self,
        label: &str,
        secret: &str,
    ) -> anyhow::Result<(Vec<WalletAccount>, WalletAccount)> {
        let label = label.trim();
        if label.is_empty() {
            bail!("label must not be empty");
        }
        let (backup, address) = local_wallet::import_local_key(secret)?;
        if let Some(existing) = self.load()?.into_iter().find(|wallet| {
            wallet.source == WalletSource::Local && wallet.address.eq_ignore_ascii_case(&address)
        }) {
            self.secrets.put(&existing.id, &backup)?;
            let mut wallet = existing;
            wallet.label = label.to_string();
            let wallets = self.upsert(wallet.clone())?;
            return Ok((wallets, wallet));
        }
        let wallet = WalletAccount {
            id: format!("local-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            label: label.to_string(),
            address,
            source: WalletSource::Local,
            env_key_name: None,
        };
        self.secrets.put(&wallet.id, &backup)?;
        let wallets = self.upsert(wallet.clone())?;
        Ok((wallets, wallet))
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    pub fn load(&self) -> anyhow::Result<Vec<WalletAccount>> {
        match std::fs::read_to_string(&self.file) {
            Ok(raw) => serde_json::from_str(&raw)
                .with_context(|| format!("could not parse {}", self.file.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, wallets: &[WalletAccount]) -> anyhow::Result<Vec<WalletAccount>> {
        for wallet in wallets {
            validate_wallet(wallet)?;
        }
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(wallets)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(wallets.to_vec())
    }

    pub fn upsert(&self, wallet: WalletAccount) -> anyhow::Result<Vec<WalletAccount>> {
        validate_wallet(&wallet)?;
        let mut wallets = self.load()?;
        if let Some(index) = wallets.iter().position(|candidate| candidate.id == wallet.id) {
            wallets[index] = wallet;
        } else {
            wallets.push(wallet);
        }
        self.save(&wallets)
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<Vec<WalletAccount>> {
        if id.trim().is_empty() {
            bail!("id must not be empty");
        }
        self.secrets.delete(id);
        let mut wallets = self.load()?;
        wallets.retain(|wallet| wallet.id != id);
        self.save(&wallets)
    }
}

fn validate_wallet(wallet: &WalletAccount) -> anyhow::Result<()> {
    if wallet.id.trim().is_empty() {
        bail!("id must not be empty");
    }
    if wallet.label.trim().is_empty() {
        bail!("label must not be empty");
    }
    for field in [&wallet.id, &wallet.label] {
        if looks_like_private_key(field) {
            bail!("{field} looks like a private key");
        }
    }
    if let Some(name) = wallet.env_key_name.as_deref()
        && looks_like_private_key(name)
    {
        bail!("env_key_name looks like a private key");
    }
    if !wallet.address.is_empty() && looks_like_private_key(&wallet.address) {
        bail!("address looks like a private key");
    }

    match wallet.source {
        WalletSource::Watch => {
            if !is_eth_address(&wallet.address) {
                bail!("watch address must match 0x + 40 hex digits");
            }
        }
        WalletSource::DevEnvKey => {
            let name = wallet
                .env_key_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("env_key_name is required"))?;
            if !is_valid_env_var_name(name) {
                bail!("env_key_name must match [A-Za-z_][A-Za-z0-9_]*");
            }
            if !wallet.address.is_empty() && !is_eth_address(&wallet.address) {
                bail!("address must be empty or match 0x + 40 hex digits");
            }
        }
        WalletSource::WalletConnect => {
            bail!("WalletConnect was removed — create or import a local key");
        }
        WalletSource::Local => {
            if !is_eth_address(&wallet.address) {
                bail!("address must match 0x + 40 hex digits");
            }
        }
    }
    Ok(())
}

fn is_eth_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_private_key(value: &str) -> bool {
    if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(label: &str, address: &str) -> WalletAccount {
        WalletAccount {
            id: "watch-1".into(),
            label: label.into(),
            address: address.into(),
            source: WalletSource::Watch,
            env_key_name: None,
        }
    }

    #[test]
    fn roundtrip_watch_address() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::new(dir.path());
        let wallet = watch("Treasury", "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01");
        store.upsert(wallet.clone()).unwrap();
        assert_eq!(store.load().unwrap(), vec![wallet]);
    }

    #[test]
    fn env_key_name_does_not_persist_key_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::new(dir.path());
        let var_name = "WAKU_TEST_WALLET_ENV_KEY";
        let secret = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        unsafe {
            std::env::set_var(var_name, secret);
        }
        store
            .upsert(WalletAccount {
                id: "env-1".into(),
                label: "Deploy key".into(),
                address: String::new(),
                source: WalletSource::DevEnvKey,
                env_key_name: Some(var_name.into()),
            })
            .unwrap();
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains(var_name));
        assert!(!raw.contains(secret));
        unsafe {
            std::env::remove_var(var_name);
        }
    }

    #[test]
    fn rejects_new_walletconnect_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::new(dir.path());
        assert!(store
            .upsert(WalletAccount {
                id: "wc-1".into(),
                label: "Phone".into(),
                address: "0x1111111111111111111111111111111111111111".into(),
                source: WalletSource::WalletConnect,
                env_key_name: None,
            })
            .is_err());
    }

    #[test]
    fn create_local_keeps_secret_out_of_address_book() {
        let dir = tempfile::tempdir().unwrap();
        let store = WalletStore::new(dir.path());
        let (wallets, wallet, backup) = store.create_local("Local").unwrap();
        assert_eq!(wallets.len(), 1);
        assert_eq!(wallet.source, WalletSource::Local);
        assert_eq!(wallet.address.len(), 42);
        assert_eq!(backup.len(), 66);
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains(&backup[2..]));
        store.remove(&wallet.id).unwrap();
        assert!(store.load().unwrap().is_empty());
    }
}
