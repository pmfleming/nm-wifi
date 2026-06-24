use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use zvariant::OwnedObjectPath;

use crate::cache;
use crate::model::{ConnectResult, WepKeyType, WifiConnectTarget};
use crate::nm::Nm;

const NMCLI_CONNECT_TIMEOUT_SECS: &str = "30";
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVATION_POLL_INTERVAL: Duration = Duration::from_millis(500);
const ACTIVATION_FAILURE_GRACE: Duration = Duration::from_secs(3);

pub(crate) fn connect_target_with_password(
    nm: &Nm,
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<ConnectResult> {
    tracing::info!(
        ssid = %target.ssid,
        ssid_len = target.ssid_bytes().len(),
        ap_path = ?target.ap_path,
        bssid = ?target.bssid,
        hidden = target.hidden,
        has_password = password.is_some(),
        wep_key_type = ?wep_key_type,
        "starting Wi-Fi connection attempt"
    );
    cache::write_status("connecting", format!("Connecting to {}…", target.ssid))?;
    match activate_saved_or_visible(nm, target, password, wep_key_type) {
        Ok(message) => {
            tracing::info!(ssid = %target.ssid, message = %message, "Wi-Fi connection succeeded");
            cache::write_status("connected", &message)?;
            refresh_cached_networks(nm)?;
            let connectivity = nm.connectivity_check().ok();
            let suggest_open_portal = target.security.as_deref() == Some("--")
                || connectivity
                    .as_ref()
                    .is_some_and(|status| status.captive_portal);
            Ok(ConnectResult {
                status: "connected",
                ssid: target.ssid.clone(),
                message,
                connectivity,
                suggest_open_portal,
            })
        }
        Err(err) => {
            tracing::error!(ssid = %target.ssid, error = %format_args!("{err:#}"), "Wi-Fi connection failed");
            cache::write_status(
                "error",
                format!("Connection failed for {}: {err:#}", target.ssid),
            )?;
            Err(err)
        }
    }
}

fn activate_saved_or_visible(
    nm: &Nm,
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<String> {
    match nm.activate_saved_wifi_connection_for(target) {
        Ok(true) => {
            tracing::info!(ssid = %target.ssid, "requested activation of saved Wi-Fi profile over D-Bus");
            wait_for_active_target(nm, target)?;
            Ok(format!(
                "Connected to saved network {} via D-Bus",
                target.ssid
            ))
        }
        Ok(false) => {
            tracing::info!(ssid = %target.ssid, "no saved D-Bus profile activation target; trying add-and-activate path");
            match nm.add_and_activate_wifi_connection_for(target, password, wep_key_type) {
                Ok(Some(created_connection)) => {
                    tracing::info!(ssid = %target.ssid, connection = %created_connection, "created and requested activation of Wi-Fi profile over D-Bus");
                    wait_for_new_connection(nm, target, &created_connection)?;
                    Ok(format!(
                        "Connected to Wi-Fi network {} via D-Bus",
                        target.ssid
                    ))
                }
                Ok(None) => {
                    tracing::info!(ssid = %target.ssid, "D-Bus add-and-activate not applicable; trying nmcli fallback");
                    activate_with_nmcli_fallback(target, password, wep_key_type)
                }
                Err(dbus_err) => {
                    tracing::warn!(ssid = %target.ssid, error = %format_args!("{dbus_err:#}"), "D-Bus add-and-activate failed; trying nmcli fallback");
                    match activate_with_nmcli_fallback(target, password, wep_key_type) {
                        Ok(message) => Ok(format!(
                            "{message} (D-Bus add/activate failed: {dbus_err:#})"
                        )),
                        Err(fallback_err) => bail!(
                            "D-Bus add/activate failed: {dbus_err:#}; nmcli fallback failed: {fallback_err:#}"
                        ),
                    }
                }
            }
        }
        Err(dbus_err) => {
            tracing::warn!(ssid = %target.ssid, error = %format_args!("{dbus_err:#}"), "D-Bus saved profile activation failed; trying nmcli fallback");
            match activate_with_nmcli_fallback(target, password, wep_key_type) {
                Ok(message) => Ok(format!("{message} (D-Bus activation failed: {dbus_err:#})")),
                Err(fallback_err) => bail!(
                    "D-Bus saved profile activation failed: {dbus_err:#}; nmcli fallback failed: {fallback_err:#}"
                ),
            }
        }
    }
}

fn wait_for_new_connection(
    nm: &Nm,
    target: &WifiConnectTarget,
    created_connection: &OwnedObjectPath,
) -> Result<()> {
    if let Err(err) = wait_for_active_target(nm, target) {
        tracing::warn!(ssid = %target.ssid, connection = %created_connection, error = %format_args!("{err:#}"), "newly-created connection failed to activate; deleting it");
        if let Err(delete_err) = nm.delete_connection(created_connection) {
            tracing::warn!(connection = %created_connection, error = %format_args!("{delete_err:#}"), "failed to delete failed newly-created connection");
            eprintln!(
                "warning: failed to delete failed connection {created_connection}: {delete_err:#}"
            );
        }
        return Err(err);
    }
    Ok(())
}

fn activate_with_nmcli_fallback(
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<String> {
    let ssid = target.ssid.as_str();
    let saved_activation = if target.has_specific_ap() {
        tracing::info!(ssid = %target.ssid, ap_path = ?target.ap_path, bssid = ?target.bssid, "skipping generic nmcli saved-profile activation for specific AP target");
        Err(anyhow::anyhow!(
            "skipped generic saved-profile activation for specific AP target"
        ))
    } else {
        tracing::info!(ssid = %target.ssid, "trying nmcli saved-profile activation fallback");
        nmcli(&["connection", "up", "id", ssid])
    };

    match saved_activation {
        Ok(_) => Ok(format!(
            "Connected to saved network {ssid} via nmcli fallback"
        )),
        Err(saved_err) => {
            if target.has_specific_ap() && target.bssid.as_deref().is_none_or(str::is_empty) {
                tracing::warn!(ssid = %target.ssid, ap_path = ?target.ap_path, "not running generic nmcli Wi-Fi connect because selected AP cannot be represented without BSSID");
                bail!(
                    "saved profile activation failed: {saved_err:#}; nmcli fallback cannot preserve selected AP path without a BSSID"
                );
            }
            let mut args = vec!["device", "wifi", "connect", ssid];
            if let Some(password) = password {
                args.extend(["password", password]);
            }
            if let Some(wep_key_type) = wep_key_type {
                args.extend(["wep-key-type", wep_key_type.nmcli_value()]);
            }
            if let Some(bssid) = target.bssid.as_deref() {
                args.extend(["bssid", bssid]);
            }
            if target.hidden {
                args.extend(["hidden", "yes"]);
            }
            match nmcli(&args) {
                Ok(_) => Ok(format!("Connected to {ssid} via nmcli fallback")),
                Err(connect_err) => bail!(
                    "saved profile activation failed: {saved_err:#}; wifi connect failed: {connect_err:#}"
                ),
            }
        }
    }
}

fn wait_for_active_target(nm: &Nm, target: &WifiConnectTarget) -> Result<()> {
    tracing::info!(ssid = %target.ssid, "waiting for target Wi-Fi network to become active");
    let deadline = Instant::now() + ACTIVATION_TIMEOUT;
    let mut saw_progress = false;
    let mut possible_failure_since = None;
    let mut last_status = None;
    while Instant::now() < deadline {
        if nm.active_ssid_matches(target)? {
            tracing::info!(ssid = %target.ssid, "target Wi-Fi network is active");
            return Ok(());
        }
        if let Some(status) = nm.wifi_activation_status_for(target)? {
            saw_progress |= status.device_state > 30;
            if status.activated() {
                tracing::debug!(
                    ssid = %target.ssid,
                    iface = %status.iface,
                    "device reports activation complete, waiting for active AP identity to match target"
                );
            }
            if saw_progress && status.terminal_failure_after_progress() {
                let failure_since = possible_failure_since.get_or_insert_with(Instant::now);
                if failure_since.elapsed() >= ACTIVATION_FAILURE_GRACE {
                    bail!(
                        "connection activation failed on {}: device state {}, reason {:?}, active connection state {:?}",
                        status.iface,
                        status.device_state,
                        status.device_state_reason,
                        status.active_connection_state
                    );
                }
            } else {
                possible_failure_since = None;
            }
            tracing::debug!(
                ssid = %target.ssid,
                iface = %status.iface,
                device_state = status.device_state,
                device_state_reason = ?status.device_state_reason,
                active_connection_state = ?status.active_connection_state,
                "activation status poll"
            );
            last_status = Some(status);
        }
        sleep(ACTIVATION_POLL_INTERVAL);
    }
    if let Some(status) = last_status {
        bail!(
            "timed out waiting for {} to become active on {}: device state {}, reason {:?}, active connection state {:?}",
            target.ssid,
            status.iface,
            status.device_state,
            status.device_state_reason,
            status.active_connection_state
        );
    }
    bail!("timed out waiting for {} to become active", target.ssid)
}

fn nmcli(args: &[&str]) -> Result<String> {
    tracing::info!(args = ?redact_nmcli_args(args), "running nmcli fallback command");
    let output = Command::new("nmcli")
        .arg("--wait")
        .arg(NMCLI_CONNECT_TIMEOUT_SECS)
        .args(args)
        .output()
        .context("run nmcli")?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        tracing::debug!(status = %output.status, stdout = %stdout, "nmcli command succeeded");
        return Ok(stdout);
    }

    let message = if stderr.is_empty() { stdout } else { stderr };
    tracing::warn!(status = %output.status, message = %message, "nmcli command failed");
    bail!("nmcli exited with {}: {message}", output.status)
}

fn redact_nmcli_args(args: &[&str]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            redacted.push("<redacted>".to_string());
            redact_next = false;
        } else {
            redacted.push((*arg).to_string());
            redact_next = *arg == "password";
        }
    }
    redacted
}

fn refresh_cached_networks(nm: &Nm) -> Result<()> {
    let networks = nm.list_access_points()?;
    cache::write_snapshot(false, &networks)?;
    cache::write_complete(false, networks.len())
}
