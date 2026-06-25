use std::collections::HashMap;

use anyhow::{Context, Result};
use zvariant::{OwnedObjectPath, OwnedValue};

use super::{ACTIVE_CONNECTION_IFACE, DEVICE_IFACE, NM_IFACE, NM_PATH, Nm, WIFI_IFACE};
use crate::model::{
    DisconnectResult, Ip4Status, SavedWifiConnection, WifiStatus, WirelessStatus, network_entries,
};

const IP4_CONFIG_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";

impl Nm {
    pub(crate) fn wifi_status(&self) -> Result<WifiStatus> {
        let profiles = self.saved_wifi_connections()?;
        let connectivity = self.connectivity_check().ok();

        for device in self.wifi_devices()? {
            let Some(active_connection_path) = self.device_active_connection_path(&device.path)?
            else {
                continue;
            };
            let Some(active_ap_path) = self.active_access_point(&device)? else {
                continue;
            };
            let access_point = self.access_point(&device.path, &active_ap_path, true)?;
            let profile = profiles
                .iter()
                .find(|profile| profile.ssid_bytes == access_point.ssid_bytes)
                .cloned()
                .or_else(|| self.active_connection_profile(&active_connection_path, &profiles));
            let active_since_ms = self
                .active_connection_timestamp(&active_connection_path)
                .or_else(|| self.active_connection_timestamp_monotonic(&active_connection_path));
            let entry = network_entries(vec![access_point.clone()], &profiles)
                .into_iter()
                .next();

            return Ok(WifiStatus {
                active: true,
                device_iface: Some(device.iface),
                active_connection_path: Some(active_connection_path.to_string()),
                access_point: Some(access_point),
                network: entry,
                profile,
                connectivity,
                ip4: self.ip4_status(&device.path).ok().flatten(),
                wireless: self.wireless_status(&device.path).ok(),
                active_since_ms,
            });
        }

        Ok(WifiStatus {
            active: false,
            device_iface: None,
            active_connection_path: None,
            access_point: None,
            network: None,
            profile: None,
            connectivity,
            ip4: None,
            wireless: None,
            active_since_ms: None,
        })
    }

    pub(crate) fn disconnect_wifi(&self) -> Result<DisconnectResult> {
        let Some(active_connection_path) = self.active_wifi_connection_path()? else {
            return Ok(DisconnectResult {
                status: "noop",
                message: "No active Wi-Fi connection".to_string(),
            });
        };

        tracing::info!(connection = %active_connection_path, "deactivating active Wi-Fi connection");
        let nm = self.proxy(NM_PATH, NM_IFACE)?;
        nm.call::<_, _, ()>("DeactivateConnection", &(active_connection_path,))
            .context("DeactivateConnection for active Wi-Fi connection")?;
        Ok(DisconnectResult {
            status: "disconnected",
            message: "Disconnected Wi-Fi".to_string(),
        })
    }

    fn active_wifi_connection_path(&self) -> Result<Option<OwnedObjectPath>> {
        for device in self.wifi_devices()? {
            if let Some(path) = self.device_active_connection_path(&device.path)? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn device_active_connection_path(
        &self,
        device_path: &OwnedObjectPath,
    ) -> Result<Option<OwnedObjectPath>> {
        let device = self.proxy_path(device_path, DEVICE_IFACE)?;
        let active_connection_path: OwnedObjectPath = device
            .get_property("ActiveConnection")
            .with_context(|| format!("read ActiveConnection for {device_path}"))?;
        Ok((active_connection_path.as_str() != "/").then_some(active_connection_path))
    }

    fn active_connection_profile(
        &self,
        active_connection_path: &OwnedObjectPath,
        profiles: &[SavedWifiConnection],
    ) -> Option<SavedWifiConnection> {
        let connection_path: OwnedObjectPath = self
            .proxy_path(active_connection_path, ACTIVE_CONNECTION_IFACE)
            .and_then(|proxy| {
                proxy
                    .get_property("Connection")
                    .context("read active profile path")
            })
            .ok()?;
        profiles
            .iter()
            .find(|profile| profile.path == connection_path.to_string())
            .cloned()
    }

    fn active_connection_timestamp(&self, active_connection_path: &OwnedObjectPath) -> Option<u64> {
        self.proxy_path(active_connection_path, ACTIVE_CONNECTION_IFACE)
            .and_then(|proxy| {
                proxy
                    .get_property("StateTimestamp")
                    .context("read StateTimestamp")
            })
            .ok()
    }

    fn active_connection_timestamp_monotonic(
        &self,
        active_connection_path: &OwnedObjectPath,
    ) -> Option<u64> {
        self.proxy_path(active_connection_path, ACTIVE_CONNECTION_IFACE)
            .and_then(|proxy| {
                proxy
                    .get_property("StateTimestampMonotonic")
                    .context("read StateTimestampMonotonic")
            })
            .ok()
    }

    fn ip4_status(&self, device_path: &OwnedObjectPath) -> Result<Option<Ip4Status>> {
        let device = self.proxy_path(device_path, DEVICE_IFACE)?;
        let ip4_config_path: OwnedObjectPath = device
            .get_property("Ip4Config")
            .with_context(|| format!("read Ip4Config for {device_path}"))?;
        if ip4_config_path.as_str() == "/" {
            return Ok(None);
        }

        let ip4 = self.proxy_path(&ip4_config_path, IP4_CONFIG_IFACE)?;
        let gateway = ip4.get_property("Gateway").ok();
        let (address, prefix) = ip4
            .get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData")
            .ok()
            .and_then(|entries| first_address_data(&entries))
            .unwrap_or((None, None));
        let dns = ip4
            .get_property::<Vec<HashMap<String, OwnedValue>>>("NameserverData")
            .ok()
            .map(|entries| nameserver_data(&entries))
            .filter(|entries| !entries.is_empty())
            .unwrap_or_default();

        Ok(Some(Ip4Status {
            address,
            prefix,
            gateway,
            dns,
        }))
    }

    fn wireless_status(&self, device_path: &OwnedObjectPath) -> Result<WirelessStatus> {
        let wifi = self.proxy_path(device_path, WIFI_IFACE)?;
        let bitrate_kbps: Option<u32> = wifi.get_property("Bitrate").ok();
        Ok(WirelessStatus {
            bitrate_mbps: bitrate_kbps.map(|value| value / 1000),
            mac_address: wifi.get_property("HwAddress").ok(),
        })
    }
}

fn first_address_data(
    entries: &[HashMap<String, OwnedValue>],
) -> Option<(Option<String>, Option<u32>)> {
    let first = entries.first()?;
    Some((
        first.get("address").and_then(value_string),
        first.get("prefix").and_then(value_u32),
    ))
}

fn nameserver_data(entries: &[HashMap<String, OwnedValue>]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| entry.get("address").and_then(value_string))
        .collect()
}

fn value_string(value: &OwnedValue) -> Option<String> {
    value.try_clone().ok()?.try_into().ok()
}

fn value_u32(value: &OwnedValue) -> Option<u32> {
    value.try_clone().ok()?.try_into().ok()
}
