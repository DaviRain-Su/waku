//! Optional CLI one-click host deploys. Tokens stay in 0600 files.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail};
use chrono::Utc;
use uuid::Uuid;
use waku_protocol::ship::{HostingProvider, HostingRecord, HostingTokenStatus};

use super::preview::detect_frontend;

const MAX_DEPLOYS: usize = 50;
const FILE_MODE: u32 = 0o600;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenFile {
    #[serde(default)]
    api_token: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

pub struct HostingStore {
    root: PathBuf,
}

impl HostingStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.to_path_buf(),
        }
    }

    fn token_path(&self, provider: HostingProvider) -> PathBuf {
        self.root.join(format!("{}-token.json", provider.id()))
    }

    fn history_path(&self) -> PathBuf {
        self.root.join("hosting.json")
    }

    fn read_token(&self, provider: HostingProvider) -> TokenFile {
        std::fs::read_to_string(self.token_path(provider))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn write_json(&self, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(FILE_MODE))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn tokens(&self) -> Vec<HostingTokenStatus> {
        [HostingProvider::Cloudflare, HostingProvider::Vercel]
            .into_iter()
            .map(|provider| self.status(provider))
            .collect()
    }

    pub fn status(&self, provider: HostingProvider) -> HostingTokenStatus {
        if let Some((token, _)) = env_token(provider) {
            return HostingTokenStatus {
                provider: provider.id().into(),
                configured: true,
                enabled: self.read_token(provider).enabled,
                key_hint: Some(mask_key(&token)),
            };
        }
        let file = self.read_token(provider);
        let token = file.api_token.trim();
        if token.is_empty() {
            return HostingTokenStatus {
                provider: provider.id().into(),
                enabled: file.enabled,
                ..HostingTokenStatus::default()
            };
        }
        HostingTokenStatus {
            provider: provider.id().into(),
            configured: true,
            enabled: file.enabled,
            key_hint: Some(mask_key(token)),
        }
    }

    pub fn set_token(
        &self,
        provider: HostingProvider,
        api_token: Option<String>,
        enabled: Option<bool>,
    ) -> anyhow::Result<Vec<HostingTokenStatus>> {
        let mut file = self.read_token(provider);
        if let Some(token) = api_token {
            let token = token.trim();
            if token.is_empty() {
                let _ = std::fs::remove_file(self.token_path(provider));
                file.api_token.clear();
            } else {
                file.api_token = token.to_string();
            }
        }
        if let Some(enabled) = enabled {
            file.enabled = enabled;
        }
        if file.api_token.is_empty() && enabled.is_none() {
            let _ = std::fs::remove_file(self.token_path(provider));
        } else {
            self.write_json(
                &self.token_path(provider),
                &serde_json::to_vec_pretty(&file)?,
            )?;
        }
        Ok(self.tokens())
    }

    pub fn access_token(&self, provider: HostingProvider) -> Option<String> {
        let file = self.read_token(provider);
        if !file.enabled {
            return None;
        }
        if let Some((token, _)) = env_token(provider) {
            return Some(token);
        }
        let token = file.api_token.trim();
        (!token.is_empty()).then(|| token.to_string())
    }

    pub fn history(&self) -> anyhow::Result<Vec<HostingRecord>> {
        match std::fs::read_to_string(self.history_path()) {
            Ok(raw) => {
                let mut records: Vec<HostingRecord> = serde_json::from_str(&raw)?;
                records.truncate(MAX_DEPLOYS);
                Ok(records)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn append(&self, record: HostingRecord) -> anyhow::Result<Vec<HostingRecord>> {
        let mut records = self.history()?;
        records.insert(0, record);
        records.truncate(MAX_DEPLOYS);
        self.write_json(&self.history_path(), &serde_json::to_vec(&records)?)?;
        Ok(records)
    }

    pub fn deploy(
        &self,
        cwd: &Path,
        provider: HostingProvider,
    ) -> anyhow::Result<HostingRecord> {
        if !cwd.is_dir() {
            bail!("cwd is not a directory: {}", cwd.display());
        }
        let detect = detect_frontend(cwd);
        if detect.kind == "none" {
            bail!("no frontend found to deploy");
        }
        let token = self.access_token(provider).ok_or_else(|| {
            anyhow!(
                "{} deploy token required — save it in Settings → MCP, or set the env var",
                match provider {
                    HostingProvider::Cloudflare => "Cloudflare",
                    HostingProvider::Vercel => "Vercel",
                }
            )
        })?;
        if !npx_available() {
            bail!("Node.js / npx not found — install Node to deploy");
        }
        let (kind, args, env_name) = match provider {
            HostingProvider::Cloudflare => {
                if detect.kind == "worker" || (detect.wrangler && detect.kind != "static") {
                    (
                        "workers",
                        vec!["--yes".into(), "wrangler".into(), "deploy".into()],
                        "CLOUDFLARE_API_TOKEN",
                    )
                } else {
                    (
                        "pages",
                        vec![
                            "--yes".into(),
                            "wrangler".into(),
                            "pages".into(),
                            "deploy".into(),
                            deploy_dir(&detect),
                            "--project-name".into(),
                            detect.project_name.clone(),
                        ],
                        "CLOUDFLARE_API_TOKEN",
                    )
                }
            }
            HostingProvider::Vercel => (
                "vercel",
                vec![
                    "--yes".into(),
                    "vercel".into(),
                    "deploy".into(),
                    "--prod".into(),
                    "--yes".into(),
                ],
                "VERCEL_TOKEN",
            ),
        };
        let output = Command::new(npx_bin())
            .args(&args)
            .current_dir(cwd)
            .env(env_name, &token)
            .env("NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| anyhow!("failed to spawn deploy: {error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            bail!("{}", first_error(&stdout, &stderr));
        }
        let url = extract_url(&stdout, &stderr).ok_or_else(|| {
            anyhow!("deploy finished but no URL was found in the CLI output")
        })?;
        let record = HostingRecord {
            id: Uuid::new_v4().to_string(),
            provider: provider.id().into(),
            kind: kind.into(),
            url,
            project_name: detect.project_name,
            cwd: Some(cwd.display().to_string()),
            ts: Utc::now().to_rfc3339(),
        };
        self.append(record.clone())?;
        Ok(record)
    }
}

fn env_token(provider: HostingProvider) -> Option<(String, &'static str)> {
    let keys = match provider {
        HostingProvider::Cloudflare => ["CLOUDFLARE_API_TOKEN", "CF_API_TOKEN"],
        HostingProvider::Vercel => ["VERCEL_TOKEN", "VERCEL_API_TOKEN"],
    };
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some((value, "env"));
            }
        }
    }
    None
}

fn deploy_dir(detect: &waku_protocol::ship::FrontendDetect) -> String {
    let root = PathBuf::from(&detect.root);
    for name in ["dist", "build", "out", "public"] {
        if root.join(name).join("index.html").is_file() {
            return name.into();
        }
    }
    if root.join("index.html").is_file() {
        return ".".into();
    }
    ".".into()
}

fn extract_url(stdout: &str, stderr: &str) -> Option<String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .rev()
        .map(str::trim)
        .find(|line| {
            line.starts_with("https://")
                && (line.contains("pages.dev")
                    || line.contains("vercel.app")
                    || line.contains("workers.dev")
                    || line.contains("cloudflare"))
        })
        .map(|line| {
            line.split_whitespace()
                .find(|token| token.starts_with("https://"))
                .unwrap_or(line)
                .trim_end_matches(['.', ',', ')'])
                .to_string()
        })
}

fn first_error(stdout: &str, stderr: &str) -> String {
    let combined = format!("{stderr}\n{stdout}");
    combined
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("deploy failed")
        .to_string()
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "····".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

fn npx_available() -> bool {
    Command::new(npx_bin())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn npx_bin() -> &'static str {
    if cfg!(windows) { "npx.cmd" } else { "npx" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip_masks_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = HostingStore::new(dir.path());
        store
            .set_token(
                HostingProvider::Cloudflare,
                Some("abcd1234efgh5678".into()),
                Some(true),
            )
            .unwrap();
        let status = store.status(HostingProvider::Cloudflare);
        assert!(status.configured);
        assert_eq!(status.key_hint.as_deref(), Some("abcd…5678"));
        let raw = std::fs::read_to_string(dir.path().join("cloudflare-token.json")).unwrap();
        assert!(raw.contains("abcd1234efgh5678"));
        assert_ne!(status.key_hint.as_deref(), Some("abcd1234efgh5678"));
    }

    #[test]
    fn extract_pages_url() {
        let log = "Compiled Worker successfully\n  https://share-registry.pages.dev\nDone";
        assert_eq!(
            extract_url(log, ""),
            Some("https://share-registry.pages.dev".into())
        );
    }
}
