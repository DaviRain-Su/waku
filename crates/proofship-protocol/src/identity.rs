//! Shared application identity used by the daemon and desktop client.

use std::path::PathBuf;

#[cfg(debug_assertions)]
pub const APP_NAME: &str = "ProofShip Debug";
#[cfg(not(debug_assertions))]
pub const APP_NAME: &str = "ProofShip";

#[cfg(debug_assertions)]
pub const APP_ID: &str = "sh.waku.dev";
#[cfg(not(debug_assertions))]
pub const APP_ID: &str = "sh.waku";

#[cfg(debug_assertions)]
pub const DATA_DIRECTORY_NAME: &str = "ProofShip Debug";
#[cfg(not(debug_assertions))]
pub const DATA_DIRECTORY_NAME: &str = "ProofShip";

#[cfg(debug_assertions)]
pub const LEGACY_DATA_DIRECTORY_NAME: &str = "Waku Debug";
#[cfg(not(debug_assertions))]
pub const LEGACY_DATA_DIRECTORY_NAME: &str = "Waku";

pub const CONFIG_DIRECTORY_NAME: &str = ".proofship";
pub const LEGACY_CONFIG_DIRECTORY_NAME: &str = ".waku";

/// Per-user application-support directory.
///
/// Prefers the ProofShip folder; if this machine still has a Waku folder and
/// ProofShip has not been created yet, keep using the legacy path so wallets
/// and transcripts are not left behind. Call [`migrate_legacy_data_directory`]
/// at process start to rename when that is safe.
pub fn data_directory() -> PathBuf {
    let root = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    preferred_directory(
        root.join(DATA_DIRECTORY_NAME),
        root.join(LEGACY_DATA_DIRECTORY_NAME),
    )
}

/// Per-user configuration directory (`~/.proofship`, or `~/.waku` when that
/// is the directory that already exists).
///
/// Projectless workspace paths are stored as absolute paths in SQLite, so this
/// never renames an existing `~/.waku` out from under them.
pub fn configuration_directory() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    preferred_directory(
        home.join(CONFIG_DIRECTORY_NAME),
        home.join(LEGACY_CONFIG_DIRECTORY_NAME),
    )
}

/// Rename `Waku` / `Waku Debug` application-support folders to ProofShip when
/// the destination does not yet exist. Safe no-op if both exist or neither
/// does.
pub fn migrate_legacy_data_directory() {
    let Some(root) = dirs::data_local_dir() else {
        return;
    };
    let current = root.join(DATA_DIRECTORY_NAME);
    let legacy = root.join(LEGACY_DATA_DIRECTORY_NAME);
    if current.exists() || !legacy.exists() {
        return;
    }
    let _ = std::fs::rename(legacy, current);
}

fn preferred_directory(current: PathBuf, legacy: PathBuf) -> PathBuf {
    if current.exists() || !legacy.exists() {
        current
    } else {
        legacy
    }
}
