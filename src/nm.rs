use std::collections::{BTreeMap, HashMap};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use zbus::blocking::{Connection, Proxy};
use zvariant::{OwnedObjectPath, Value};

use crate::model::{AccessPoint, WifiDevice, security_label};

pub(crate) const NM_DEST: &str = "org.freedesktop.NetworkManager";
pub(crate) const WIFI_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);

const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_DEVICE_TYPE_WIFI: u32 = 2;

pub(crate) struct Nm {
    conn: Connection,
}

impl Nm {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            conn: Connection::system().context("connect to system D-Bus")?,
        })
    }

    pub(crate) fn connection(&self) -> Connection {
        self.conn.clone()
    }

    fn proxy<'a>(&'a self, path: &'a str, iface: &'a str) -> Result<Proxy<'a>> {
        Proxy::new(&self.conn, NM_DEST, path, iface).context("create D-Bus proxy")
    }

    fn proxy_path<'a>(&'a self, path: &'a OwnedObjectPath, iface: &'a str) -> Result<Proxy<'a>> {
        self.proxy(path.as_str(), iface)
    }

    pub(crate) fn wifi_devices(&self) -> Result<Vec<WifiDevice>> {
        let nm = self.proxy(NM_PATH, NM_IFACE)?;
        let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).context("GetDevices")?;
        devices
            .into_iter()
            .filter_map(|path| self.wifi_device(path).transpose())
            .collect()
    }

    fn wifi_device(&self, path: OwnedObjectPath) -> Result<Option<WifiDevice>> {
        let device = self.proxy_path(&path, DEVICE_IFACE)?;
        let device_type: u32 = device
            .get_property("DeviceType")
            .with_context(|| format!("read DeviceType for {path}"))?;
        if device_type != NM_DEVICE_TYPE_WIFI {
            return Ok(None);
        }
        let iface = device
            .get_property("Interface")
            .unwrap_or_else(|_| path.to_string());
        drop(device);
        Ok(Some(WifiDevice { path, iface }))
    }

    pub(crate) fn active_ssid(&self) -> Result<Option<String>> {
        for device in self.wifi_devices()? {
            let Some(active_path) = self.active_access_point(&device)? else {
                continue;
            };
            return self
                .access_point(&active_path, true)
                .map(|ap| Some(ap.ssid));
        }
        Ok(None)
    }

    pub(crate) fn list_access_points(&self) -> Result<Vec<AccessPoint>> {
        let mut by_ssid = BTreeMap::new();
        for device in self.wifi_devices()? {
            self.add_device_access_points(&device, &mut by_ssid)?;
        }
        Ok(sorted_access_points(by_ssid))
    }

    fn add_device_access_points(
        &self,
        device: &WifiDevice,
        by_ssid: &mut BTreeMap<String, AccessPoint>,
    ) -> Result<()> {
        let active_path = self.active_access_point(device)?;
        for path in self.device_access_points(device)? {
            let active = active_path.as_ref().is_some_and(|active| *active == path);
            if let Some(ap) = self.read_visible_access_point(&path, active) {
                merge_access_point(by_ssid, ap);
            }
        }
        Ok(())
    }

    fn active_access_point(&self, device: &WifiDevice) -> Result<Option<OwnedObjectPath>> {
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        let active_path: OwnedObjectPath = wifi
            .get_property("ActiveAccessPoint")
            .with_context(|| format!("read ActiveAccessPoint for {}", device.iface))?;
        Ok((active_path.as_str() != "/").then_some(active_path))
    }

    fn device_access_points(&self, device: &WifiDevice) -> Result<Vec<OwnedObjectPath>> {
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        wifi.call("GetAccessPoints", &())
            .with_context(|| format!("GetAccessPoints for {}", device.iface))
    }

    fn read_visible_access_point(
        &self,
        path: &OwnedObjectPath,
        active: bool,
    ) -> Option<AccessPoint> {
        match self.access_point(path, active) {
            Ok(ap) if !ap.ssid.is_empty() => Some(ap),
            Ok(_) => None,
            Err(err) => {
                eprintln!("warning: skipping access point {path}: {err:#}");
                None
            }
        }
    }

    fn access_point(&self, path: &OwnedObjectPath, active: bool) -> Result<AccessPoint> {
        let ap = self.proxy_path(path, AP_IFACE)?;
        let ssid_bytes: Vec<u8> = ap
            .get_property("Ssid")
            .with_context(|| format!("read Ssid for {path}"))?;
        let flags = ap.get_property("Flags").unwrap_or(0);
        let wpa_flags = ap.get_property("WpaFlags").unwrap_or(0);
        let rsn_flags = ap.get_property("RsnFlags").unwrap_or(0);

        Ok(AccessPoint {
            ssid: String::from_utf8_lossy(&ssid_bytes).into_owned(),
            active,
            security: security_label(flags, wpa_flags, rsn_flags),
            strength: ap.get_property("Strength").unwrap_or(0),
            frequency: ap.get_property("Frequency").unwrap_or(0),
            bssid: ap.get_property("HwAddress").unwrap_or_default(),
            last_seen: ap.get_property("LastSeen").unwrap_or(-1),
        })
    }

    pub(crate) fn scan(&self, timeout: Duration) -> Result<()> {
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
        let before = self.last_scan(device);
        self.request_scan(device)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.last_scan_completed(device, before) {
                return Ok(());
            }
            sleep(POLL_INTERVAL);
        }
        bail!("timed out waiting for LastScan to change")
    }

    pub(crate) fn request_scan(&self, device: &WifiDevice) -> Result<()> {
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        let options = HashMap::<&str, Value<'_>>::new();
        wifi.call::<_, _, ()>("RequestScan", &(options,))
            .context("RequestScan")
    }

    pub(crate) fn last_scan(&self, device: &WifiDevice) -> i64 {
        self.proxy_path(&device.path, WIFI_IFACE)
            .and_then(|wifi| wifi.get_property("LastScan").context("read LastScan"))
            .unwrap_or(-1)
    }

    fn last_scan_completed(&self, device: &WifiDevice, before: i64) -> bool {
        let after = self.last_scan(device);
        after != before && after >= 0
    }
}

fn merge_access_point(by_ssid: &mut BTreeMap<String, AccessPoint>, ap: AccessPoint) {
    by_ssid
        .entry(ap.ssid.clone())
        .and_modify(|existing| {
            if ap.active || (!existing.active && ap.strength > existing.strength) {
                *existing = ap.clone();
            }
        })
        .or_insert(ap);
}

fn sorted_access_points(by_ssid: BTreeMap<String, AccessPoint>) -> Vec<AccessPoint> {
    let mut aps: Vec<_> = by_ssid.into_values().collect();
    aps.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then_with(|| b.strength.cmp(&a.strength))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    aps
}
