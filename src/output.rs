use std::io::{self, Write};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::model::AccessPoint;

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
        networks: &'a [AccessPoint],
    },
    Complete {
        timed_out: bool,
        networks_found: usize,
    },
}

pub(crate) fn print_access_points(aps: &[AccessPoint]) {
    for ap in aps {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            ap.ssid,
            if ap.active { "*" } else { "" },
            ap.security,
            ap.strength,
            ap.frequency,
            ap.bssid,
            ap.last_seen,
        );
    }
}

pub(crate) fn emit_stream_event(event: &StreamOutput<'_>) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer(&mut stdout, event).context("write JSON event")?;
    stdout.write_all(b"\n").context("write JSON newline")?;
    stdout.flush().context("flush JSON event")?;
    Ok(())
}
