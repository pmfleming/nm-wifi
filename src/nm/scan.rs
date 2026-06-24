use std::collections::HashMap;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use zvariant::Value;

use super::{Nm, POLL_INTERVAL, WIFI_IFACE};
use crate::model::WifiDevice;

impl Nm {
    pub(crate) fn scan(&self, timeout: Duration) -> Result<()> {
        let devices = self.wifi_devices()?;
        tracing::info!(
            device_count = devices.len(),
            timeout_secs = timeout.as_secs(),
            "starting blocking Wi-Fi scan"
        );
        if devices.is_empty() {
            bail!("no Wi-Fi devices found");
        }
        for device in devices {
            self.scan_device(&device, timeout)
                .with_context(|| format!("scan {}", device.iface))?;
        }
        tracing::info!("blocking Wi-Fi scan completed");
        Ok(())
    }

    fn scan_device(&self, device: &WifiDevice, timeout: Duration) -> Result<()> {
        let before = self.last_scan(device);
        tracing::debug!(iface = %device.iface, before, "requesting blocking scan for device");
        self.request_scan(device)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.last_scan_completed(device, before) {
                tracing::debug!(iface = %device.iface, after = self.last_scan(device), "device scan completed");
                return Ok(());
            }
            sleep(POLL_INTERVAL);
        }
        bail!("timed out waiting for LastScan to change")
    }

    pub(crate) fn request_scan(&self, device: &WifiDevice) -> Result<()> {
        tracing::info!(iface = %device.iface, path = %device.path, "requesting NetworkManager scan");
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        let options = HashMap::<&str, Value<'_>>::new();
        wifi.call::<_, _, ()>("RequestScan", &(options,))
            .context("RequestScan")
    }

    pub(super) fn request_hidden_scan(&self, device: &WifiDevice, ssid_bytes: &[u8]) -> Result<()> {
        tracing::info!(iface = %device.iface, ssid_len = ssid_bytes.len(), "requesting targeted hidden SSID scan");
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        let options = HashMap::from([("ssids", Value::new(vec![ssid_bytes.to_vec()]))]);
        wifi.call::<_, _, ()>("RequestScan", &(options,))
            .with_context(|| format!("RequestScan hidden SSID on {}", device.iface))
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
