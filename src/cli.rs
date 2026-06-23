use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nm-wifi-rofi")]
#[command(about = "NetworkManager D-Bus Wi-Fi helper for rofi")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// List visible Wi-Fi networks as TSV.
    List,
    /// Request a scan, wait for completion, then list visible Wi-Fi networks as TSV.
    Scan {
        /// Scan completion timeout in seconds.
        #[arg(long, default_value_t = 12)]
        timeout: u64,
        /// Emit JSON Lines snapshots while NetworkManager discovers access points.
        #[arg(long)]
        stream: bool,
        /// Return an error instead of printing cached results when scan fails.
        #[arg(long)]
        strict: bool,
        /// Number of scan request retries when NetworkManager rejects a request.
        #[arg(long, default_value_t = 2)]
        retries: u32,
    },
    /// Print the active SSID, if any.
    Active,
}
