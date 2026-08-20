//! Digest-pinned ProofForge compiler install under the daemon data dir.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail};
use proofship_protocol::web3::PfToolchainStatus;
use sha2::{Digest, Sha256};

pub const PINNED_VERSION: &str = "0.1.1";
const PINNED_URL_DARWIN_ARM64: &str = "https://github.com/DaviRain-Su/proof_forge/releases/download/v0.1.1/proof-forge-next-0.1.1-darwin-arm64.tar.gz";
const PINNED_SHA256_DARWIN_ARM64: &str =
    "489a7ae55c171d8d8ab14e4bcfa688129184342af77588e96ea01de8c9c43a4a";

static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
static STATE: OnceLock<Mutex<InstallState>> = OnceLock::new();

struct InstallState {
    installing: bool,
    last: PfToolchainStatus,
}

pub fn init(data_dir: &Path) {
    *ROOT.lock().unwrap_or_else(|error| error.into_inner()) =
        Some(data_dir.join("toolchains").join("proof-forge"));
}

fn prefix() -> Option<PathBuf> {
    ROOT.lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

fn state() -> &'static Mutex<InstallState> {
    STATE.get_or_init(|| {
        Mutex::new(InstallState {
            installing: false,
            last: detect_status(),
        })
    })
}

fn lock_state() -> std::sync::MutexGuard<'static, InstallState> {
    state().lock().unwrap_or_else(|error| error.into_inner())
}

pub fn status() -> PfToolchainStatus {
    let guard = lock_state();
    if guard.installing {
        return installing_status(guard.last.error.clone());
    }
    guard.last.clone()
}

pub fn start_install() -> PfToolchainStatus {
    let snapshot = detect_status();
    if snapshot.state == "ready" {
        let mut guard = lock_state();
        guard.last = snapshot.clone();
        return snapshot;
    }
    if !platform_supported() {
        let status = PfToolchainStatus {
            state: "missing".into(),
            error: Some("ProofForge is only packaged for Apple Silicon Macs".into()),
            ..PfToolchainStatus::default()
        };
        lock_state().last = status.clone();
        return status;
    }
    {
        let mut guard = lock_state();
        if guard.installing {
            return installing_status(None);
        }
        guard.installing = true;
        guard.last = installing_status(None);
    }
    thread::Builder::new()
        .name("waku-pf-install".into())
        .spawn(run_install)
        .map_err(|error| anyhow!("could not start install: {error}"))
        .ok();
    installing_status(None)
}

pub fn uninstall() -> anyhow::Result<PfToolchainStatus> {
    if lock_state().installing {
        bail!("install is still running");
    }
    if let Some(root) = prefix() {
        let _ = std::fs::remove_dir_all(&root);
    }
    let status = detect_status();
    lock_state().last = status.clone();
    Ok(status)
}

pub fn resolve_cli() -> Option<PathBuf> {
    host_cli().or_else(managed_cli)
}

pub fn managed_cli() -> Option<PathBuf> {
    let root = prefix()?;
    cli_under(&root.join("current")).or_else(|| cli_under(&root.join(PINNED_VERSION)))
}

pub fn host_cli() -> Option<PathBuf> {
    env_file("PF_CLI")
        .or_else(|| env_file("PROOF_FORGE_CLI"))
        .or_else(|| find_on_path("proof-forge-next"))
        .or_else(|| {
            env_dir("PROOF_FORGE_ROOT").and_then(|root| {
                let built = root.join(".lake/build/bin/proof-forge-next");
                built
                    .is_file()
                    .then_some(built)
                    .or_else(|| cli_under(&root))
            })
        })
}

pub fn cli_env() -> Vec<(String, String)> {
    let Some(cli) = resolve_cli() else {
        return Vec::new();
    };
    let mut env = vec![("PROOF_FORGE_CLI".into(), cli.display().to_string())];
    if let Some(root) = cli_root(&cli) {
        env.push(("PROOF_FORGE_ROOT".into(), root.display().to_string()));
        env.push(("WAKU_PROOF_FORGE_ROOT".into(), root.display().to_string()));
    }
    if let Some(tools) = prefix().map(|root| root.join("tools")) {
        env.push(("PROOF_FORGE_TOOL_ROOT".into(), tools.display().to_string()));
    }
    env
}

pub fn detect_status() -> PfToolchainStatus {
    if let Some(cli) = host_cli() {
        return ready_status(&cli, "host");
    }
    if let Some(cli) = managed_cli() {
        return ready_status(&cli, "managed");
    }
    PfToolchainStatus {
        state: "missing".into(),
        ..PfToolchainStatus::default()
    }
}

fn ready_status(cli: &Path, source: &str) -> PfToolchainStatus {
    let version = read_version(cli).or_else(|| Some(PINNED_VERSION.into()));
    PfToolchainStatus {
        state: "ready".into(),
        version,
        cli: Some(cli.display().to_string()),
        source: Some(source.into()),
        evm_ready: evm_tools_ready(cli),
        error: None,
    }
}

fn installing_status(error: Option<String>) -> PfToolchainStatus {
    PfToolchainStatus {
        state: "installing".into(),
        version: Some(PINNED_VERSION.into()),
        error,
        ..PfToolchainStatus::default()
    }
}

fn run_install() {
    let result = install_pinned();
    let mut guard = lock_state();
    guard.installing = false;
    guard.last = match result {
        Ok(()) => detect_status(),
        Err(error) => PfToolchainStatus {
            state: "missing".into(),
            error: Some(error.to_string()),
            ..PfToolchainStatus::default()
        },
    };
}

fn install_pinned() -> anyhow::Result<()> {
    if host_cli().is_some() {
        return Ok(());
    }
    if managed_cli().is_some() {
        return Ok(());
    }
    let prefix = prefix().ok_or_else(|| anyhow!("toolchain store is not initialized"))?;
    let url = pinned_url()?;
    let expected = pinned_sha256()?;
    std::fs::create_dir_all(&prefix)?;
    let tmp = prefix.join(format!("download-{PINNED_VERSION}.tar.gz"));
    download_file(url, &tmp)?;
    verify_sha256(&tmp, expected)?;
    let staging = prefix.join(format!("staging-{PINNED_VERSION}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    extract_archive(&tmp, &staging)?;
    let unpacked = find_unpacked_root(&staging)
        .ok_or_else(|| anyhow!("archive did not contain proof-forge-next"))?;
    let dest = prefix.join(PINNED_VERSION);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&unpacked, &dest).or_else(|_| copy_tree(&unpacked, &dest))?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&tmp);
    link_current(&prefix, PINNED_VERSION)?;
    if let Some(cli) = cli_under(&dest) {
        let _ = setup_evm(&cli);
    }
    Ok(())
}

fn pinned_url() -> anyhow::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(PINNED_URL_DARWIN_ARM64),
        _ => bail!("ProofForge is only packaged for Apple Silicon Macs"),
    }
}

fn pinned_sha256() -> anyhow::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(PINNED_SHA256_DARWIN_ARM64),
        _ => bail!("ProofForge is only packaged for Apple Silicon Macs"),
    }
}

fn platform_supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

fn download_file(url: &str, dest: &Path) -> anyhow::Result<()> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("waku-pf-install")
        .build()?
        .get(url)
        .send()
        .map_err(|error| anyhow!("download failed: {error}"))?;
    if !response.status().is_success() {
        bail!("download failed: HTTP {}", response.status());
    }
    let bytes = response
        .bytes()
        .map_err(|error| anyhow!("download failed: {error}"))?;
    std::fs::write(dest, bytes)?;
    Ok(())
}

pub fn verify_sha256(path: &Path, expected: &str) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got = format!("{:x}", hasher.finalize());
    if got != expected {
        bail!("sha256 mismatch: got {got} want {expected}");
    }
    Ok(())
}

pub fn extract_archive(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let status = Command::new("tar")
        .args([
            "-xzf",
            &archive.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
        .status()
        .map_err(|error| anyhow!("tar failed: {error}"))?;
    if !status.success() {
        bail!("tar exited {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn find_unpacked_root(staging: &Path) -> Option<PathBuf> {
    if cli_under(staging).is_some() {
        return Some(staging.to_path_buf());
    }
    let entries = std::fs::read_dir(staging).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && cli_under(&path).is_some() {
            return Some(path);
        }
    }
    None
}

fn link_current(prefix: &Path, version: &str) -> anyhow::Result<()> {
    let current = prefix.join("current");
    let _ = std::fs::remove_file(&current);
    let _ = std::fs::remove_dir_all(&current);
    #[cfg(unix)]
    std::os::unix::fs::symlink(version, &current)?;
    #[cfg(not(unix))]
    {
        let _ = version;
        bail!("symlink is required to activate the toolchain");
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn cli_under(root: &Path) -> Option<PathBuf> {
    for candidate in [
        root.join("bin/proof-forge-next"),
        root.join("proof-forge-next"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn cli_root(cli: &Path) -> Option<PathBuf> {
    let parent = cli.parent()?;
    if parent.file_name().is_some_and(|name| name == "bin") {
        return parent.parent().map(Path::to_path_buf);
    }
    Some(parent.to_path_buf())
}

fn read_version(cli: &Path) -> Option<String> {
    if let Some(root) = cli_root(cli) {
        let file = root.join("VERSION");
        if let Ok(raw) = std::fs::read_to_string(file) {
            let version = raw.trim();
            if !version.is_empty() {
                return Some(version.to_string());
            }
        }
    }
    let output = Command::new(cli)
        .args(["version", "--json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn setup_evm(cli: &Path) -> anyhow::Result<()> {
    let mut command = Command::new(cli);
    command.args(["install", "--targets", "evm", "--yes"]);
    if let Some(root) = cli_root(cli) {
        command.env("PROOF_FORGE_ROOT", &root);
        command.env("PROOF_FORGE_CLI", cli);
    }
    if let Some(tools) = prefix().map(|root| root.join("tools")) {
        let _ = std::fs::create_dir_all(&tools);
        command.env("PROOF_FORGE_TOOL_ROOT", tools);
    }
    let status = command
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|error| anyhow!("evm setup failed: {error}"))?;
    if !status.success() {
        bail!("evm setup exited {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn evm_tools_ready(cli: &Path) -> bool {
    let mut command = Command::new(cli);
    command.args(["doctor", "--target", "evm", "--json"]);
    if let Some(root) = cli_root(cli) {
        command.env("PROOF_FORGE_ROOT", root);
        command.env("PROOF_FORGE_CLI", cli);
    }
    let output = match command.output() {
        Ok(output) => output,
        Err(_) => return false,
    };
    if output.status.success() {
        return true;
    }
    find_on_path("solc").is_some()
}

fn env_file(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_rejects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, b"hello").unwrap();
        let error = verify_sha256(&path, "00").unwrap_err().to_string();
        assert!(error.contains("sha256 mismatch"));
    }

    #[test]
    fn sha256_accepts_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, b"hello").unwrap();
        verify_sha256(
            &path,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        )
        .unwrap();
    }

    #[test]
    fn detects_managed_prefix() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        let version = dir
            .path()
            .join("toolchains/proof-forge")
            .join(PINNED_VERSION);
        std::fs::create_dir_all(version.join("bin")).unwrap();
        std::fs::write(version.join("bin/proof-forge-next"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                version.join("bin/proof-forge-next"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let prefix_dir = dir.path().join("toolchains/proof-forge");
        link_current(&prefix_dir, PINNED_VERSION).unwrap();
        assert!(
            cli_under(&version).is_some(),
            "cli missing under {}",
            version.display()
        );
        assert_eq!(prefix().as_deref(), Some(prefix_dir.as_path()));
        let cli = managed_cli().expect("managed cli");
        assert!(cli.ends_with("bin/proof-forge-next"));
    }

    #[test]
    fn extract_finds_nested_cli() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("src");
        let nested = staging.join("proof-forge-next-0.1.1-darwin-arm64/bin");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("proof-forge-next"), b"ok").unwrap();
        let archive = dir.path().join("pf.tar.gz");
        let status = Command::new("tar")
            .current_dir(dir.path())
            .args([
                "-czf",
                &archive.to_string_lossy(),
                "-C",
                "src",
                "proof-forge-next-0.1.1-darwin-arm64",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        extract_archive(&archive, &dest).unwrap();
        let root = find_unpacked_root(&dest).expect("root");
        assert!(cli_under(&root).is_some());
    }

    #[test]
    fn host_cli_wins_over_managed() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        let host = dir.path().join("host-cli");
        std::fs::write(&host, b"host").unwrap();
        let previous = std::env::var_os("PROOF_FORGE_CLI");
        unsafe { std::env::set_var("PROOF_FORGE_CLI", &host) };
        let resolved = resolve_cli();
        match previous {
            Some(value) => unsafe { std::env::set_var("PROOF_FORGE_CLI", value) },
            None => unsafe { std::env::remove_var("PROOF_FORGE_CLI") },
        }
        assert_eq!(resolved.as_deref(), Some(host.as_path()));
    }
}
