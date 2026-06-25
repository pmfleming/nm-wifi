use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
    let Some(snapshot) = read_json::<CachedSnapshot>(snapshot_path())? else {
        return Ok(None);
    };
    if snapshot.version != CACHE_VERSION {
        warn_cache_ignored(format!(
            "cache version {} is stale; expected {CACHE_VERSION}",
            snapshot.version
        ));
        return Ok(None);
    }
    Ok(Some(snapshot))
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
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            warn_cache_ignored(format!("could not read {}: {err}", path.display()));
            return Ok(None);
        }
    };
    match serde_json::from_str(&text) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            warn_cache_ignored(format!("could not parse {}: {err}", path.display()));
            Ok(None)
        }
    }
}

fn write_json<T>(path: PathBuf, value: &T) -> Result<()>
where
    T: Serialize,
{
    let parent = path.parent().context("cache path has no parent")?;
    create_private_dir_all(parent)?;
    let tmp_path = temp_path_for(&path)?;
    let text = serde_json::to_string_pretty(value).context("serialize cache JSON")?;
    write_private_file(&tmp_path, format!("{text}\n").as_bytes())
        .with_context(|| format!("write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))
}

fn temp_path_for(path: &Path) -> Result<PathBuf> {
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

pub(crate) fn create_private_dir_all(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        create_private_dir_all_unix(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
    }
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_dir_all_unix(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match fs::symlink_metadata(path) {
        Ok(link_metadata) => {
            if link_metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "refusing to use symlinked cache directory {}",
                    path.display()
                );
            }
            let metadata =
                fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
            if !metadata.is_dir() {
                anyhow::bail!("{} exists but is not a directory", path.display());
            }
            let current_uid = current_euid();
            if metadata.uid() != current_uid {
                anyhow::bail!(
                    "refusing to use {} owned by uid {}; expected uid {}",
                    path.display(),
                    metadata.uid(),
                    current_uid
                );
            }
            if metadata.mode() & 0o077 != 0 {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .with_context(|| format!("chmod 0700 {}", path.display()))?;
            }
            return Ok(());
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("lstat {}", path.display())),
    }

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(path)
        .with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", path.display()))
}

#[cfg(unix)]
fn current_euid() -> u32 {
    unsafe { geteuid() }
}

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
}

fn warn_cache_ignored(message: String) {
    tracing::warn!(message = %message, "ignoring Wi-Fi cache");
    eprintln!("warning: ignoring Wi-Fi cache: {message}");
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
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime_dir) => PathBuf::from(runtime_dir).join(CACHE_DIR_NAME),
        None => std::env::temp_dir().join(format!("{CACHE_DIR_NAME}-{}", current_user_id())),
    }
}

fn current_user_id() -> u32 {
    #[cfg(unix)]
    {
        current_euid()
    }
    #[cfg(not(unix))]
    {
        0
    }
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
