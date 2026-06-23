use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::AccessPoint;

const CACHE_VERSION: u32 = 1;
const CACHE_DIR_NAME: &str = "nm-wifi-rofi";
const REVEAL_INTERVAL_MS: u128 = 10;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CachedSnapshot {
    version: u32,
    updated_at_ms: u128,
    scanning: bool,
    networks_found: usize,
    networks: Vec<AccessPoint>,
}

impl CachedSnapshot {
    pub(crate) fn scanning(&self) -> bool {
        self.scanning
    }

    pub(crate) fn updated_at_ms(&self) -> u128 {
        self.updated_at_ms
    }

    pub(crate) fn networks_found(&self) -> usize {
        self.networks_found
    }

    pub(crate) fn networks(&self) -> &[AccessPoint] {
        &self.networks
    }

    pub(crate) fn into_networks(self) -> Vec<AccessPoint> {
        self.networks
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RevealState {
    active: bool,
    visible_count: usize,
    last_reveal_ms: u128,
    source_updated_at_ms: u128,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CachedStatus {
    version: u32,
    updated_at_ms: u128,
    state: String,
    message: String,
    timed_out: Option<bool>,
    networks_found: Option<usize>,
}

impl CachedStatus {
    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

pub(crate) fn reset_progressive_reveal() -> Result<()> {
    write_json(
        reveal_path(),
        &RevealState {
            active: true,
            visible_count: 0,
            last_reveal_ms: 0,
            source_updated_at_ms: 0,
        },
    )
}

pub(crate) fn visible_network_count(
    scanning: bool,
    snapshot_updated_at_ms: u128,
    available: usize,
) -> Result<usize> {
    let Some(mut reveal) = read_reveal()? else {
        return Ok(available);
    };
    if !reveal.active {
        return Ok(available);
    }

    reveal.visible_count = reveal.visible_count.min(available);
    if should_reveal_next(&reveal, available) {
        reveal.visible_count += 1;
        reveal.last_reveal_ms = now_ms();
        reveal.source_updated_at_ms = snapshot_updated_at_ms;
    }
    reveal.active = scanning || reveal.visible_count < available;
    let visible_count = reveal.visible_count;
    write_json(reveal_path(), &reveal)?;
    Ok(visible_count)
}

fn should_reveal_next(reveal: &RevealState, available: usize) -> bool {
    available > reveal.visible_count
        && (reveal.visible_count == 0
            || now_ms().saturating_sub(reveal.last_reveal_ms) >= REVEAL_INTERVAL_MS)
}

pub(crate) fn write_empty_scanning_snapshot() -> Result<()> {
    write_session_snapshot(true, &[])
}

pub(crate) fn write_live_scan_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    if scanning {
        return write_session_snapshot(true, networks);
    }
    write_snapshot(false, networks)?;
    write_session_snapshot(false, networks)
}

pub(crate) fn write_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    let snapshot = CachedSnapshot {
        version: CACHE_VERSION,
        updated_at_ms: now_ms(),
        scanning,
        networks_found: networks.len(),
        networks: networks.to_vec(),
    };
    write_json(snapshot_path(), &snapshot)
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

pub(crate) fn read_session_snapshot() -> Result<Option<CachedSnapshot>> {
    read_json(session_path())
}

pub(crate) fn read_status() -> Result<Option<CachedStatus>> {
    read_json(status_path())
}

fn read_reveal() -> Result<Option<RevealState>> {
    read_json(reveal_path())
}

fn write_session_snapshot(scanning: bool, networks: &[AccessPoint]) -> Result<()> {
    let snapshot = CachedSnapshot {
        version: CACHE_VERSION,
        updated_at_ms: now_ms(),
        scanning,
        networks_found: networks.len(),
        networks: networks.to_vec(),
    };
    write_json(session_path(), &snapshot)
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
    let tmp_path = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(value).context("serialize cache JSON")?;
    fs::write(&tmp_path, format!("{text}\n"))
        .with_context(|| format!("write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))
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

fn reveal_path() -> PathBuf {
    cache_dir().join("reveal.json")
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(CACHE_DIR_NAME)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
