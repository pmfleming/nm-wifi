use std::collections::BTreeMap;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use zbus::blocking::{Connection, Proxy};
use zvariant::{OwnedObjectPath, Value};

const NM_DEST: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIFI_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_DEVICE_TYPE_WIFI: u32 = 2;
const NM_AP_FLAGS_PRIVACY: u32 = 0x1;

#[derive(Parser)]
#[command(name = "nm-wifi-rofi")]
#[command(about = "NetworkManager D-Bus Wi-Fi helper for rofi")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List visible Wi-Fi networks as TSV.
    List,
    /// Request a scan, wait for completion, then list visible Wi-Fi networks as TSV.
    Scan {
        /// Scan completion timeout in seconds.
        #[arg(long, default_value_t = 12)]
        timeout: u64,
    },
    /// Print the active SSID, if any.
    Active,
}

#[derive(Debug, Clone)]
struct WifiDevice {
    path: OwnedObjectPath,
    iface: String,
}

#[derive(Debug, Clone)]
struct AccessPoint {
    ssid: String,
    active: bool,
    security: String,
    strength: u8,
    frequency: u32,
    bssid: String,
    last_seen: i32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let nm = Nm::new()?;

    match cli.command {
        Command::List => print_access_points(&nm.list_access_points()?),
        Command::Scan { timeout } => {
            nm.scan(Duration::from_secs(timeout))?;
            print_access_points(&nm.list_access_points()?);
        }
        Command::Active => {
            if let Some(ssid) = nm.active_ssid()? {
                println!("{ssid}");
            }
        }
    }

    Ok(())
}

struct Nm {
    conn: Connection,
}

impl Nm {
    fn new() -> Result<Self> {
        Ok(Self {
            conn: Connection::system().context("connect to system D-Bus")?,
        })
    }

    fn proxy<'a>(&'a self, path: &'a str, iface: &'a str) -> Result<Proxy<'a>> {
        Proxy::new(&self.conn, NM_DEST, path, iface).context("create D-Bus proxy")
    }

    fn proxy_path<'a>(&'a self, path: &'a OwnedObjectPath, iface: &'a str) -> Result<Proxy<'a>> {
        self.proxy(path.as_str(), iface)
    }

    fn wifi_devices(&self) -> Result<Vec<WifiDevice>> {
        let nm = self.proxy(NM_PATH, NM_IFACE)?;
        let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).context("GetDevices")?;

        let mut wifi = Vec::new();
        for path in devices {
            let device = self.proxy_path(&path, DEVICE_IFACE)?;
            let device_type: u32 = device
                .get_property("DeviceType")
                .with_context(|| format!("read DeviceType for {path}"))?;
            if device_type != NM_DEVICE_TYPE_WIFI {
                continue;
            }

            let iface: String = device
                .get_property("Interface")
                .unwrap_or_else(|_| path.to_string());
            drop(device);
            wifi.push(WifiDevice { path, iface });
        }

        Ok(wifi)
    }

    fn active_ssid(&self) -> Result<Option<String>> {
        for device in self.wifi_devices()? {
            let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
            let active_path: OwnedObjectPath = wifi
                .get_property("ActiveAccessPoint")
                .with_context(|| format!("read ActiveAccessPoint for {}", device.iface))?;
            if active_path.as_str() == "/" {
                continue;
            }
            let ap = self.access_point(&active_path, true)?;
            return Ok(Some(ap.ssid));
        }
        Ok(None)
    }

    fn list_access_points(&self) -> Result<Vec<AccessPoint>> {
        let mut by_ssid: BTreeMap<String, AccessPoint> = BTreeMap::new();

        for device in self.wifi_devices()? {
            let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
            let active_path: OwnedObjectPath = wifi
                .get_property("ActiveAccessPoint")
                .unwrap_or_else(|_| OwnedObjectPath::try_from("/").expect("valid object path"));
            let aps: Vec<OwnedObjectPath> = wifi
                .call("GetAccessPoints", &())
                .with_context(|| format!("GetAccessPoints for {}", device.iface))?;

            for path in aps {
                let active = path == active_path;
                let ap = self.access_point(&path, active)?;
                if ap.ssid.is_empty() {
                    continue;
                }

                by_ssid
                    .entry(ap.ssid.clone())
                    .and_modify(|existing| {
                        if ap.active || (!existing.active && ap.strength > existing.strength) {
                            *existing = ap.clone();
                        }
                    })
                    .or_insert(ap);
            }
        }

        let mut aps: Vec<_> = by_ssid.into_values().collect();
        aps.sort_by(|a, b| {
            b.active
                .cmp(&a.active)
                .then_with(|| b.strength.cmp(&a.strength))
                .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
        });
        Ok(aps)
    }

    fn access_point(&self, path: &OwnedObjectPath, active: bool) -> Result<AccessPoint> {
        let ap = self.proxy_path(path, AP_IFACE)?;
        let ssid_bytes: Vec<u8> = ap
            .get_property("Ssid")
            .with_context(|| format!("read Ssid for {path}"))?;
        let ssid = String::from_utf8_lossy(&ssid_bytes).into_owned();
        let strength: u8 = ap.get_property("Strength").unwrap_or(0);
        let flags: u32 = ap.get_property("Flags").unwrap_or(0);
        let wpa_flags: u32 = ap.get_property("WpaFlags").unwrap_or(0);
        let rsn_flags: u32 = ap.get_property("RsnFlags").unwrap_or(0);
        let frequency: u32 = ap.get_property("Frequency").unwrap_or(0);
        let bssid: String = ap.get_property("HwAddress").unwrap_or_default();
        let last_seen: i32 = ap.get_property("LastSeen").unwrap_or(-1);

        Ok(AccessPoint {
            ssid,
            active,
            security: security_label(flags, wpa_flags, rsn_flags),
            strength,
            frequency,
            bssid,
            last_seen,
        })
    }

    fn scan(&self, timeout: Duration) -> Result<()> {
        let devices = self.wifi_devices()?;
        if devices.is_empty() {
            bail!("no Wi-Fi devices found");
        }

        for device in devices {
            self.scan_device(&device, timeout)
                .with_context(|| format!("scan {}", device.iface))?;
        }
        Ok(())
    }

    fn scan_device(&self, device: &WifiDevice, timeout: Duration) -> Result<()> {
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        let before: i64 = wifi.get_property("LastScan").unwrap_or(-1);
        let options = std::collections::HashMap::<&str, Value<'_>>::new();
        wifi.call::<_, _, ()>("RequestScan", &(options,))
            .context("RequestScan")?;

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let after: i64 = wifi.get_property("LastScan").unwrap_or(before);
            if after != before && after >= 0 {
                return Ok(());
            }
            sleep(Duration::from_millis(250));
        }

        bail!("timed out waiting for LastScan to change")
    }
}

fn print_access_points(aps: &[AccessPoint]) {
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

fn security_label(flags: u32, wpa_flags: u32, rsn_flags: u32) -> String {
    if flags & NM_AP_FLAGS_PRIVACY == 0 && wpa_flags == 0 && rsn_flags == 0 {
        "--".to_string()
    } else if rsn_flags != 0 {
        "WPA2/3".to_string()
    } else if wpa_flags != 0 {
        "WPA".to_string()
    } else {
        "WEP".to_string()
    }
}
