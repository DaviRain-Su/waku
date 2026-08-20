//! Local EVM network presets persisted under `{data_dir}/web3/networks.json`.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use proofship_protocol::web3::{
    EvmNetwork, Web3Prefs, builtin_networks, default_network_id, is_builtin_network_id,
};

pub struct NetworkStore {
    file: PathBuf,
}

impl NetworkStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("networks.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    pub fn load(&self) -> anyhow::Result<Vec<EvmNetwork>> {
        match std::fs::read_to_string(&self.file) {
            Ok(raw) => {
                let networks: Vec<EvmNetwork> = serde_json::from_str(&raw)
                    .with_context(|| format!("could not parse {}", self.file.display()))?;
                Ok(merge_builtins(networks))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(builtin_networks()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, networks: &[EvmNetwork]) -> anyhow::Result<Vec<EvmNetwork>> {
        let merged = merge_builtins(networks.to_vec());
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&merged)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(merged)
    }

    pub fn upsert(&self, network: EvmNetwork) -> anyhow::Result<Vec<EvmNetwork>> {
        validate_network(&network)?;
        let mut network = network;
        if is_builtin_network_id(&network.id) {
            network.builtin = true;
            if let Some(preset) = builtin_networks()
                .into_iter()
                .find(|candidate| candidate.id == network.id)
            {
                network.chain_id = preset.chain_id;
            }
        }
        let mut networks = self.load()?;
        if let Some(index) = networks
            .iter()
            .position(|candidate| candidate.id == network.id)
        {
            networks[index] = network;
        } else {
            networks.push(network);
        }
        self.save(&networks)
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<Vec<EvmNetwork>> {
        if is_builtin_network_id(id) {
            bail!("cannot remove built-in network {id}");
        }
        let mut networks = self.load()?;
        networks.retain(|network| network.id != id);
        self.save(&networks)
    }
}

fn merge_builtins(mut networks: Vec<EvmNetwork>) -> Vec<EvmNetwork> {
    for network in &mut networks {
        if is_builtin_network_id(&network.id) {
            network.builtin = true;
        }
    }
    for builtin in builtin_networks() {
        if !networks.iter().any(|network| network.id == builtin.id) {
            networks.push(builtin);
        }
    }
    networks
}

pub struct PrefsStore {
    file: PathBuf,
}

impl PrefsStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("prefs.json"),
        }
    }

    pub fn load(&self) -> anyhow::Result<Web3Prefs> {
        match std::fs::read_to_string(&self.file) {
            Ok(raw) => serde_json::from_str(&raw)
                .with_context(|| format!("could not parse {}", self.file.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Web3Prefs::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, prefs: Web3Prefs) -> anyhow::Result<Web3Prefs> {
        let mut prefs = prefs;
        if prefs.selected_network_id.trim().is_empty() {
            prefs.selected_network_id = default_network_id().to_string();
        }
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&prefs)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(prefs)
    }
}

fn validate_network(network: &EvmNetwork) -> anyhow::Result<()> {
    if network.id.is_empty()
        || !network
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("id must be non-empty and match [a-z0-9-]+");
    }
    if network.name.trim().is_empty() {
        bail!("name must be non-empty");
    }
    if network.chain_id == 0 {
        bail!("chainId must be greater than 0");
    }
    if !network.rpc_url.starts_with("http://") && !network.rpc_url.starts_with("https://") {
        bail!("rpcUrl must start with http:// or https://");
    }
    if network.currency_symbol.trim().is_empty() {
        bail!("currencySymbol must be non-empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom() -> EvmNetwork {
        EvmNetwork {
            id: "my-net".into(),
            name: "My Net".into(),
            chain_id: 42,
            rpc_url: "https://rpc.example.com".into(),
            explorer_url: Some("https://explorer.example.com".into()),
            currency_symbol: "ETH".into(),
            builtin: false,
            enabled: true,
        }
    }

    #[test]
    fn missing_file_yields_xlayer_first_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        let loaded = store.load().unwrap();
        assert!(loaded.iter().all(|network| network.builtin));
        assert_eq!(loaded[0].id, "xlayer-mainnet");
        assert_eq!(loaded[1].id, "xlayer-testnet");
        assert!(
            loaded
                .iter()
                .any(|network| network.id == "ethereum-sepolia")
        );
        assert!(loaded.iter().any(|network| network.id == "base-sepolia"));
        let anvil = loaded
            .iter()
            .find(|network| network.id == "anvil")
            .expect("anvil");
        assert!(!anvil.enabled);
        assert_eq!(anvil.chain_id, 31337);
    }

    #[test]
    fn cannot_remove_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        let error = store.remove("xlayer-testnet").unwrap_err();
        assert!(error.to_string().contains("built-in"));
    }

    #[test]
    fn enabled_toggle_persists_for_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        let mut mainnet = store
            .load()
            .unwrap()
            .into_iter()
            .find(|network| network.id == "xlayer-mainnet")
            .unwrap();
        mainnet.enabled = false;
        store.upsert(mainnet).unwrap();
        let reloaded = store.load().unwrap();
        let mainnet = reloaded
            .iter()
            .find(|network| network.id == "xlayer-mainnet")
            .unwrap();
        assert!(!mainnet.enabled);
        assert!(mainnet.builtin);
    }

    #[test]
    fn builtin_upsert_keeps_preset_chain_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        let mut mainnet = builtin_networks()
            .into_iter()
            .find(|network| network.id == "xlayer-mainnet")
            .unwrap();
        mainnet.chain_id = 1;
        mainnet.rpc_url = "https://example.invalid".into();
        store.upsert(mainnet).unwrap();
        let row = store
            .load()
            .unwrap()
            .into_iter()
            .find(|network| network.id == "xlayer-mainnet")
            .unwrap();
        assert_eq!(row.chain_id, 196);
        assert_eq!(row.rpc_url, "https://example.invalid");
    }

    #[test]
    fn custom_network_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        store.upsert(custom()).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.iter().any(|network| network.id == "my-net"));
        assert_eq!(loaded.len(), 6);
    }

    #[test]
    fn saved_list_without_anvil_appends_it_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let store = NetworkStore::new(dir.path());
        let without_anvil: Vec<EvmNetwork> = builtin_networks()
            .into_iter()
            .filter(|network| network.id != "anvil")
            .collect();
        std::fs::write(store.path(), serde_json::to_vec(&without_anvil).unwrap()).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.last().map(|network| network.id.as_str()),
            Some("anvil")
        );
        assert!(!loaded.last().unwrap().enabled);
        assert_eq!(loaded[0].id, "xlayer-mainnet");
    }
}
