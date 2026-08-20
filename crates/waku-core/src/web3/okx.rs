//! OKX OnchainOS API key store. The raw key never appears in RPC status.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use waku_protocol::web3::OkxStatus;

pub const ONCHAINOS_MCP_URL: &str = "https://web3.okx.com/api/v1/onchainos-mcp";
pub const ONCHAINOS_MCP_NAME: &str = "okx-onchainos";
pub const ACCESS_KEY_HEADER: &str = "OK-ACCESS-KEY";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxConfig {
    #[serde(default)]
    api_key: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for OkxConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            enabled: true,
        }
    }
}

pub struct OkxStore {
    file: PathBuf,
}

impl OkxStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join("okx.json"),
        }
    }

    fn read(&self) -> OkxConfig {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn write(&self, config: &OkxConfig) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(config)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    pub fn get(&self) -> Option<String> {
        let key = self.read().api_key.trim().to_string();
        (!key.is_empty()).then_some(key)
    }

    pub fn enabled(&self) -> bool {
        self.read().enabled
    }

    pub fn put(&self, api_key: &str) -> std::io::Result<()> {
        let key = api_key.trim();
        if key.is_empty() {
            let _ = std::fs::remove_file(&self.file);
            return Ok(());
        }
        self.write(&OkxConfig {
            api_key: key.to_string(),
            enabled: true,
        })
    }

    pub fn set_enabled(&self, enabled: bool) -> std::io::Result<()> {
        let mut config = self.read();
        config.enabled = enabled;
        self.write(&config)
    }

    pub fn status(&self) -> OkxStatus {
        let enabled = self.enabled();
        match access_key_from(Some(self)) {
            Some((key, source)) => OkxStatus {
                configured: true,
                enabled,
                source: Some(source.into()),
                key_hint: Some(mask_key(&key)),
            },
            None => OkxStatus {
                enabled,
                ..OkxStatus::default()
            },
        }
    }
}

static STORE: OnceLock<OkxStore> = OnceLock::new();

pub fn init(data_dir: &Path) {
    let _ = STORE.set(OkxStore::new(data_dir));
}

fn access_key_from(store: Option<&OkxStore>) -> Option<(String, &'static str)> {
    for var in ["OKX_ONCHAINOS_API_KEY", "OK_ACCESS_KEY"] {
        if let Ok(value) = std::env::var(var) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some((value, "env"));
            }
        }
    }
    let key = store.or_else(|| STORE.get()).and_then(OkxStore::get)?;
    Some((key, "stored"))
}

pub fn access_key() -> Option<String> {
    if env_truthy("WAKU_DISABLE_OKX_MCP") || env_truthy("PROOFSHIP_DISABLE_OKX_MCP") {
        return None;
    }
    if STORE.get().is_some_and(|store| !store.enabled()) {
        return None;
    }
    access_key_from(None).map(|(key, _)| key)
}

pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "····".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
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
    fn store_round_trip_and_mask() {
        let dir = tempfile::tempdir().unwrap();
        let store = OkxStore::new(dir.path());
        assert!(store.get().is_none());
        assert!(!store.status().configured);
        store.put("  abcd1234efgh5678  ").unwrap();
        assert_eq!(store.get().as_deref(), Some("abcd1234efgh5678"));
        let status = store.status();
        assert!(status.configured);
        assert_eq!(status.key_hint.as_deref(), Some("abcd…5678"));
        let raw = std::fs::read_to_string(dir.path().join("okx.json")).unwrap();
        assert!(raw.contains("abcd1234efgh5678"));
        assert_ne!(status.key_hint.as_deref(), Some("abcd1234efgh5678"));
        store.put("").unwrap();
        assert!(store.get().is_none());
    }

    #[test]
    fn short_keys_are_fully_masked() {
        assert_eq!(mask_key("abc"), "····");
        assert_eq!(mask_key("abcd1234x"), "abcd…234x");
    }
}
