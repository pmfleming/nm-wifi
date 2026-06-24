use std::io::{self, Write};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::model::{
    AccessPoint, ConnectResult, ConnectivityStatus, NetworkEntry, SavedWifiConnection,
};

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub(crate) enum StreamOutput<'a> {
    Status {
        message: String,
    },
    Warning {
        message: String,
    },
    Snapshot {
        scanning: bool,
        networks_found: usize,
        networks: &'a [NetworkEntry],
    },
    Complete {
        timed_out: bool,
        networks_found: usize,
    },
}

pub(crate) fn print_access_points_json(aps: &[AccessPoint]) -> Result<()> {
    let text = serde_json::to_string_pretty(aps).context("serialize AP JSON")?;
    println!("{text}");
    Ok(())
}

pub(crate) fn print_network_entries_json(networks: &[NetworkEntry]) -> Result<()> {
    let text = serde_json::to_string_pretty(networks).context("serialize network JSON")?;
    println!("{text}");
    Ok(())
}

pub(crate) fn print_saved_wifi_connections_json(profiles: &[SavedWifiConnection]) -> Result<()> {
    let text = serde_json::to_string_pretty(profiles).context("serialize saved Wi-Fi JSON")?;
    println!("{text}");
    Ok(())
}

pub(crate) fn print_connect_result(result: &ConnectResult, json: bool) -> Result<()> {
    if json {
        let text = serde_json::to_string_pretty(result).context("serialize connect result JSON")?;
        println!("{text}");
    } else {
        println!("{}", result.message);
    }
    Ok(())
}

pub(crate) fn print_connectivity(status: &ConnectivityStatus, json: bool) -> Result<()> {
    if json {
        let text = serde_json::to_string_pretty(status).context("serialize connectivity JSON")?;
        println!("{text}");
    } else {
        println!("{}", status.state);
    }
    Ok(())
}

pub(crate) fn print_saved_wifi_connections(profiles: &[SavedWifiConnection]) {
    for profile in profiles {
        println!(
            "{}\t{}\t{}\t{}",
            tsv_escape(&profile.id),
            tsv_escape(&profile.ssid),
            if profile.autoconnect {
                "autoconnect"
            } else {
                "manual"
            },
            tsv_escape(&profile.path),
        );
    }
}

pub(crate) fn print_access_points(aps: &[AccessPoint]) {
    for ap in aps {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            tsv_escape(&ap.ssid),
            if ap.active { "*" } else { "" },
            tsv_escape(&ap.security),
            ap.strength,
            ap.frequency,
            tsv_escape(&ap.bssid),
            ap.last_seen,
        );
    }
}

fn tsv_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\0' => escaped.push_str("\\0"),
            ch if ch.is_control() => escaped.push_str(&format!("\\x{:02x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

pub(crate) fn emit_stream_event(event: &StreamOutput<'_>) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer(&mut stdout, event).context("write JSON event")?;
    stdout.write_all(b"\n").context("write JSON newline")?;
    stdout.flush().context("flush JSON event")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::tsv_escape;

    #[test]
    fn tsv_escape_preserves_row_shape() {
        assert_eq!(
            tsv_escape("Cafe\\Wi-Fi\tline\nnull\0\x1f"),
            "Cafe\\\\Wi-Fi\\tline\\nnull\\0\\x1f"
        );
    }
}
