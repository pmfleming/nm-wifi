use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use crate::cli::{Cli, Command, ProfileCommand};
use crate::model::{ScanStreamOptions, WifiConnectTarget, network_entries};
use crate::nm::Nm;
use crate::output::{
    print_access_points, print_access_points_json, print_connect_result, print_connectivity,
    print_network_entries_json, print_saved_wifi_connections, print_saved_wifi_connections_json,
};

mod cache;
mod cli;
mod connect;
mod logging;
mod model;
mod nm;
mod output;
mod stream;
mod stream_emit;
mod stream_watch;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_path = logging::init(cli.verbose, cli.log_file.clone())?;
    tracing::debug!(path = %log_path.display(), "using log file");
    let nm = Nm::new()?;

    match cli.command {
        Command::List {
            json,
            cached,
            refresh_cache,
            refresh_timeout,
        } => print_network_list(&nm, json, cached, refresh_cache, refresh_timeout)?,
        Command::Networks {
            json,
            cached,
            refresh_cache,
            refresh_timeout,
        } => print_enriched_network_list(&nm, json, cached, refresh_cache, refresh_timeout)?,
        Command::Scan {
            timeout,
            stream,
            strict,
            retries,
            cache,
        } => run_scan(&nm, timeout, stream, strict, retries, cache)?,
        Command::Connect {
            ssid,
            password,
            bssid,
            hidden,
            wep_key_type,
            json,
        } => print_connect_result(
            &connect::connect_target_with_password(
                &nm,
                &WifiConnectTarget {
                    ssid_bytes: ssid.as_bytes().to_vec(),
                    ssid,
                    ap_path: None,
                    bssid,
                    hidden,
                    security: None,
                },
                password.as_deref(),
                wep_key_type,
            )?,
            json,
        )?,
        Command::ConnectTarget {
            target_json,
            password,
            wep_key_type,
            json,
        } => print_connect_result(
            &connect::connect_target_with_password(
                &nm,
                &parse_connect_target(&target_json)?,
                password.as_deref(),
                wep_key_type,
            )?,
            json,
        )?,
        Command::Saved { json } => print_saved_profiles(&nm, json)?,
        Command::Profile { command } => run_profile_command(&nm, command)?,
        Command::Connectivity { json } => print_connectivity(&nm.connectivity_check()?, json)?,
        Command::Active => print_active_ssid(&nm)?,
    }

    Ok(())
}

fn parse_connect_target(target_json: &str) -> Result<WifiConnectTarget> {
    serde_json::from_str(target_json).context("parse Wi-Fi connect target JSON")
}

fn print_network_list(
    nm: &Nm,
    json: bool,
    cached: bool,
    refresh_cache: bool,
    refresh_timeout: u64,
) -> Result<()> {
    tracing::info!(
        json,
        cached,
        refresh_cache,
        refresh_timeout,
        "listing Wi-Fi networks"
    );

    if cached {
        if let Some(snapshot) = cache::read_snapshot()? {
            let networks = snapshot.into_networks();
            if refresh_cache {
                spawn_cache_refresh(refresh_timeout);
            }
            return print_networks(&networks, json);
        }

        if refresh_cache {
            tracing::info!(
                refresh_timeout,
                "no cached scan exists; refreshing cache before listing"
            );
            let networks = scan_and_cache(nm, Duration::from_secs(refresh_timeout))?;
            return print_networks(&networks, json);
        }
    }

    let networks = nm.list_access_points()?;
    if refresh_cache {
        spawn_cache_refresh(refresh_timeout);
    }
    print_networks(&networks, json)
}

fn print_networks(networks: &[crate::model::AccessPoint], json: bool) -> Result<()> {
    if json {
        print_access_points_json(networks)
    } else {
        print_access_points(networks);
        Ok(())
    }
}

fn print_enriched_network_list(
    nm: &Nm,
    json: bool,
    cached: bool,
    refresh_cache: bool,
    refresh_timeout: u64,
) -> Result<()> {
    let access_points = load_networks(nm, cached, refresh_cache, refresh_timeout)?;
    let profiles = nm.saved_wifi_connections()?;
    let networks = network_entries(access_points, &profiles);
    if json {
        print_network_entries_json(&networks)
    } else {
        print_access_points(
            &networks
                .into_iter()
                .map(|network| network.access_point)
                .collect::<Vec<_>>(),
        );
        Ok(())
    }
}

fn load_networks(
    nm: &Nm,
    cached: bool,
    refresh_cache: bool,
    refresh_timeout: u64,
) -> Result<Vec<crate::model::AccessPoint>> {
    if cached {
        if let Some(snapshot) = cache::read_snapshot()? {
            let networks = snapshot.into_networks();
            if refresh_cache {
                spawn_cache_refresh(refresh_timeout);
            }
            return Ok(networks);
        }

        if refresh_cache {
            return scan_and_cache(nm, Duration::from_secs(refresh_timeout));
        }
    }

    let networks = nm.list_access_points()?;
    if refresh_cache {
        spawn_cache_refresh(refresh_timeout);
    }
    Ok(networks)
}

fn scan_and_cache(nm: &Nm, timeout: Duration) -> Result<Vec<crate::model::AccessPoint>> {
    if let Err(err) = nm.scan(timeout) {
        tracing::warn!(error = %format_args!("{err:#}"), "cache refresh scan failed before list");
        eprintln!("warning: scan failed: {err:#}; showing cached NetworkManager results");
    }
    let networks = nm.list_access_points()?;
    cache::write_snapshot(false, &networks)?;
    cache::write_complete(false, networks.len())?;
    Ok(networks)
}

fn spawn_cache_refresh(timeout: u64) {
    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            tracing::warn!(error = %err, "could not find current executable for background cache refresh");
            return;
        }
    };

    let timeout_arg = timeout.to_string();
    match ProcessCommand::new(current_exe)
        .args(["scan", "--cache", "--timeout", timeout_arg.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => tracing::info!(
            pid = child.id(),
            timeout,
            "spawned background Wi-Fi cache refresh"
        ),
        Err(err) => {
            tracing::warn!(error = %err, timeout, "failed to spawn background Wi-Fi cache refresh")
        }
    }
}

fn run_scan(
    nm: &Nm,
    timeout: u64,
    stream: bool,
    strict: bool,
    retries: u32,
    cache: bool,
) -> Result<()> {
    tracing::info!(
        timeout,
        stream,
        strict,
        retries,
        cache,
        "running Wi-Fi scan"
    );
    let timeout = Duration::from_secs(timeout);
    if stream {
        return nm.scan_stream(ScanStreamOptions {
            timeout,
            retries,
            cache,
        });
    }

    if let Err(err) = nm.scan(timeout) {
        tracing::warn!(error = %format_args!("{err:#}"), "scan failed");
        if strict {
            return Err(err);
        }
        eprintln!("warning: scan failed: {err:#}; showing cached NetworkManager results");
    }
    let networks = nm.list_access_points()?;
    if cache {
        cache::write_snapshot(false, &networks)?;
        cache::write_complete(false, networks.len())?;
    }
    print_access_points(&networks);
    Ok(())
}

fn print_saved_profiles(nm: &Nm, json: bool) -> Result<()> {
    tracing::info!(json, "listing saved Wi-Fi profiles");
    let profiles = nm.saved_wifi_connections()?;
    if json {
        print_saved_wifi_connections_json(&profiles)
    } else {
        print_saved_wifi_connections(&profiles);
        Ok(())
    }
}

fn run_profile_command(nm: &Nm, command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::Delete { path } => {
            tracing::info!(path, "deleting saved Wi-Fi profile");
            nm.delete_connection_by_path(&path)?;
        }
        ProfileCommand::Autoconnect { path, enabled } => {
            tracing::info!(path, enabled, "setting saved Wi-Fi profile autoconnect");
            nm.set_connection_autoconnect_by_path(&path, enabled)?;
        }
    }
    Ok(())
}

fn print_active_ssid(nm: &Nm) -> Result<()> {
    if let Some(ssid) = nm.active_ssid()? {
        println!("{ssid}");
    }
    Ok(())
}
