use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::AccessPoint;

const CACHE_VERSION: u32 = 1;
const CACHE_DIR_NAME: &str = "nm-wifi";

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CachedSnapshot {
    version: u32,
    updated_at_ms: u128,
    scanning: bool,
    networks_found: usize,
    networks: Vec<AccessPoint>,
}

impl CachedSnapshot {
    pub(crate) fn into_networks(self) -> Vec<AccessPoint> {
        self.networks
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CachedStatus {
    version: u32,
    updated_at_ms: u128,
    state: String,
    message: String,
    timed_out: Option<bool>,
    networks_found: Option<usize>,
}

pub(crate) fn write_live_scan_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    if !scanning {
        write_snapshot(false, networks)?;
    }
    write_session_snapshot(scanning, networks)
}

pub(crate) fn write_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    write_snapshot_to(snapshot_path(), scanning, networks)
}

pub(crate) fn write_status(state: impl Into<String>, message: impl Into<String>) -> Result<()> {
    write_status_record(CachedStatus {
        version: CACHE_VERSION,
        updated_at_ms: now_ms(),
        state: state.into(),
        message: message.into(),
        timed_out: None,
        networks_found: None,
    })
}

pub(crate) fn write_complete(timed_out: bool, networks_found: usize) -> Result<()> {
    let message = if timed_out {
        format!("scan timed out; {networks_found} networks available")
    } else {
        format!("scan complete; {networks_found} networks available")
    };
    write_status_record(CachedStatus {
        version: CACHE_VERSION,
        updated_at_ms: now_ms(),
        state: "complete".to_string(),
        message,
        timed_out: Some(timed_out),
        networks_found: Some(networks_found),
    })
}

pub(crate) fn read_snapshot() -> Result<Option<CachedSnapshot>> {
    read_json(snapshot_path())
}

fn write_session_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    write_snapshot_to(session_path(), scanning, networks)
}

fn write_snapshot_to(path: PathBuf, scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    write_json(path, &snapshot_record(scanning, networks))
}

fn snapshot_record(scanning: bool, networks: &[AccessPoint]) -> CachedSnapshot {
    CachedSnapshot {
        version: CACHE_VERSION,
        updated_at_ms: now_ms(),
        scanning,
        networks_found: networks.len(),
        networks: networks.to_vec(),
    }
}

fn write_status_record(status: CachedStatus) -> Result<()> {
    write_json(status_path(), &status)
}

fn read_json<T>(path: PathBuf) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parse {}", path.display()))
        .map(Some)
}

fn write_json<T>(path: PathBuf, value: &T) -> Result<()>
where
    T: Serialize,
{
    let parent = path.parent().context("cache path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp_path = temp_path_for(&path)?;
    let text = serde_json::to_string_pretty(value).context("serialize cache JSON")?;
    fs::write(&tmp_path, format!("{text}\n"))
        .with_context(|| format!("write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))
}

fn temp_path_for(path: &std::path::Path) -> Result<PathBuf> {
    let parent = path.parent().context("cache path has no parent")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("cache path has no file name")?;
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    )))
}

fn snapshot_path() -> PathBuf {
    cache_dir().join("latest.json")
}

fn status_path() -> PathBuf {
    cache_dir().join("status.json")
}

fn session_path() -> PathBuf {
    cache_dir().join("scan-session.json")
}

pub(crate) fn log_path() -> PathBuf {
    cache_dir().join("nm-wifi.log")
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(CACHE_DIR_NAME)
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::temp_path_for;

    #[test]
    fn temp_paths_are_unique_for_same_cache_path() {
        let path = PathBuf::from("/tmp/nm-wifi/status.json");

        let first = temp_path_for(&path).expect("first temp path");
        let second = temp_path_for(&path).expect("second temp path");

        assert_ne!(first, second);
        assert_eq!(first.parent(), path.parent());
        assert_eq!(second.parent(), path.parent());
    }
}
