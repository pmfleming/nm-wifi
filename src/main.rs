use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};
use crate::model::ScanStreamOptions;
use crate::nm::Nm;
use crate::output::print_access_points;

mod cli;
mod model;
mod nm;
mod output;
mod stream;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let nm = Nm::new()?;

    match cli.command {
        Command::List => print_access_points(&nm.list_access_points()?),
        Command::Scan {
            timeout,
            stream,
            strict,
            retries,
        } => run_scan(&nm, timeout, stream, strict, retries)?,
        Command::Active => print_active_ssid(&nm)?,
    }

    Ok(())
}

fn run_scan(nm: &Nm, timeout: u64, stream: bool, strict: bool, retries: u32) -> Result<()> {
    let timeout = Duration::from_secs(timeout);
    if stream {
        return nm.scan_stream(ScanStreamOptions { timeout, retries });
    }

    if let Err(err) = nm.scan(timeout) {
        if strict {
            return Err(err);
        }
        eprintln!("warning: scan failed: {err:#}; showing cached NetworkManager results");
    }
    print_access_points(&nm.list_access_points()?);
    Ok(())
}

fn print_active_ssid(nm: &Nm) -> Result<()> {
    if let Some(ssid) = nm.active_ssid()? {
        println!("{ssid}");
    }
    Ok(())
}
