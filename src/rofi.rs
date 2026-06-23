use std::env;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::cache::{self, CachedSnapshot};
use crate::model::AccessPoint;
use crate::nm::Nm;

const ACTION_RESCAN: &str = "rescan";
const ACTION_STATUS: &str = "status";
const ACTION_SSID_PREFIX: &str = "ssid:";
const ROFI_CUSTOM_RESCAN_OR_REFRESH: &str = "10";

pub(crate) fn run(nm: &Nm, timeout: u64, retries: u32) -> Result<()> {
    handle_action(nm, timeout, retries)?;
    emit_menu(nm)
}

fn handle_action(nm: &Nm, timeout: u64, retries: u32) -> Result<()> {
    if is_custom_rescan_or_refresh() {
        return handle_rescan_hotkey(timeout, retries);
    }

    match selected_action().as_deref() {
        Some(ACTION_RESCAN) => request_background_scan(timeout, retries),
        Some(ACTION_STATUS) | None => Ok(()),
        Some(action) => handle_network_action(nm, action),
    }
}

fn is_custom_rescan_or_refresh() -> bool {
    env::var("ROFI_RETV").as_deref() == Ok(ROFI_CUSTOM_RESCAN_OR_REFRESH)
}

fn handle_rescan_hotkey(timeout: u64, retries: u32) -> Result<()> {
    if cache::read_snapshot()?.is_some_and(|snapshot| snapshot.scanning()) {
        return Ok(());
    }
    request_background_scan(timeout, retries)
}

fn selected_action() -> Option<String> {
    env::var("ROFI_INFO").ok().filter(|value| !value.is_empty())
}

fn request_background_scan(timeout: u64, retries: u32) -> Result<()> {
    cache::write_empty_scanning_snapshot()?;
    cache::write_status("scanning", "Scanning… 0 networks found")?;
    start_background_scan(timeout, retries)
}

fn handle_network_action(nm: &Nm, action: &str) -> Result<()> {
    let Some(ssid) = action.strip_prefix(ACTION_SSID_PREFIX) else {
        return Ok(());
    };
    if let Err(err) = crate::connect::connect_ssid(nm, ssid) {
        eprintln!("warning: {err:#}");
    }
    Ok(())
}

fn start_background_scan(timeout: u64, retries: u32) -> Result<()> {
    let timeout = timeout.to_string();
    let retries = retries.to_string();
    Command::new(env::current_exe().context("find current executable")?)
        .args([
            "scan",
            "--stream",
            "--cache",
            "--timeout",
            &timeout,
            "--retries",
            &retries,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start background cached scan")?;
    Ok(())
}

fn emit_menu(nm: &Nm) -> Result<()> {
    print_rofi_header();
    let snapshot = cache::read_snapshot()?;
    print_rescan_row(snapshot.as_ref());
    if snapshot
        .as_ref()
        .is_none_or(|snapshot| !snapshot.scanning())
    {
        print_status_row(snapshot.as_ref())?;
    }

    for ap in menu_networks(nm, snapshot)? {
        print_network_row(&ap);
    }
    Ok(())
}

fn print_rescan_row(snapshot: Option<&CachedSnapshot>) {
    match snapshot.filter(|snapshot| snapshot.scanning()) {
        Some(snapshot) => print_disabled_row(scan_progress_label(snapshot), ACTION_STATUS),
        None => print_row(" Rescan", ACTION_RESCAN),
    }
}

fn scan_progress_label(snapshot: &CachedSnapshot) -> String {
    format!(
        " Scanning… {} networks found — Alt+R refreshes",
        snapshot.networks_found()
    )
}

fn menu_networks(nm: &Nm, snapshot: Option<CachedSnapshot>) -> Result<Vec<AccessPoint>> {
    if let Some(snapshot) = snapshot {
        return Ok(snapshot.into_networks());
    }
    nm.list_access_points()
}

fn print_status_row(snapshot: Option<&CachedSnapshot>) -> Result<()> {
    if let Some(status) = cache::read_status()? {
        print_row(clean_label(status.message()), ACTION_STATUS);
    } else if let Some(snapshot) = snapshot {
        print_row(
            format!("Cached: {} networks", snapshot.networks_found()),
            ACTION_STATUS,
        );
    } else {
        print_row("No cached scan yet", ACTION_STATUS);
    }
    Ok(())
}

fn print_network_row(ap: &AccessPoint) {
    let active = if ap.active { "●" } else { " " };
    let lock = if ap.security == "--" { " " } else { "" };
    let label = format!(
        "{active} {lock} {:>3}%  {}",
        ap.strength,
        clean_label(&ap.ssid)
    );
    print_row(label, format!("ssid:{}", ap.ssid));
}

fn print_rofi_header() {
    println!("\0prompt\x1fWi-Fi");
    println!("\0no-custom\x1ftrue");
    println!("\0use-hot-keys\x1ftrue");
    println!("\0keep-selection\x1ftrue");
    println!("\0keep-filter\x1ftrue");
}

fn print_row(label: impl AsRef<str>, info: impl AsRef<str>) {
    println!("{}\0info\x1f{}", label.as_ref(), info.as_ref());
}

fn print_disabled_row(label: impl AsRef<str>, info: impl AsRef<str>) {
    println!(
        "{}\0info\x1f{}\x1fnonselectable\x1ftrue",
        label.as_ref(),
        info.as_ref()
    );
}

fn clean_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\t' | '\n' | '\r' | '\0' => ' ',
            _ => ch,
        })
        .collect()
}
