use std::collections::BTreeMap;

use anyhow::{Context, Result};
use zvariant::OwnedObjectPath;

use super::{AP_IFACE, DEVICE_IFACE, NM_DEVICE_TYPE_WIFI, NM_IFACE, NM_PATH, Nm, WIFI_IFACE};
use crate::model::{AccessPoint, WifiConnectTarget, WifiDevice, display_ssid, security_label};

impl Nm {
    pub(crate) fn wifi_devices(&self) -> Result<Vec<WifiDevice>> {
        let nm = self.proxy(NM_PATH, NM_IFACE)?;
        let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).context("GetDevices")?;
        let wifi_devices: Vec<_> = devices
            .into_iter()
            .filter_map(|path| self.wifi_device(path).transpose())
            .collect::<Result<_>>()?;
        tracing::debug!(count = wifi_devices.len(), "discovered Wi-Fi devices");
        Ok(wifi_devices)
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
        tracing::debug!(path = %path, iface = %iface, "found Wi-Fi device");
        Ok(Some(WifiDevice { path, iface }))
    }

    pub(crate) fn active_ssid(&self) -> Result<Option<String>> {
        for device in self.wifi_devices()? {
            let Some(active_path) = self.active_access_point(&device)? else {
                continue;
            };
            return self
                .access_point(&device.path, &active_path, true)
                .map(|ap| Some(ap.ssid));
        }
        Ok(None)
    }

    pub(crate) fn active_ssid_matches(&self, target: &WifiConnectTarget) -> Result<bool> {
        let target_ssid = target.ssid_bytes();
        for device in self.wifi_devices()? {
            let Some(active_path) = self.active_access_point(&device)? else {
                continue;
            };
            let ap = self.access_point(&device.path, &active_path, true)?;
            if access_point_matches(
                &ap,
                target_ssid.as_ref(),
                target.ap_path.as_deref(),
                target.bssid.as_deref(),
            ) {
                tracing::debug!(ssid = %target.ssid, bssid = %ap.bssid, ap_path = %ap.path, "target access point is active");
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn visible_access_point_for(
        &self,
        target: &WifiConnectTarget,
    ) -> Result<Option<(WifiDevice, OwnedObjectPath, AccessPoint)>> {
        let target_ssid = target.ssid_bytes();
        for device in self.wifi_devices()? {
            for path in self.device_access_points(&device)? {
                let Ok(ap) = self.access_point(&device.path, &path, false) else {
                    continue;
                };
                if access_point_matches(
                    &ap,
                    target_ssid.as_ref(),
                    target.ap_path.as_deref(),
                    target.bssid.as_deref(),
                ) {
                    tracing::debug!(
                        ssid = %target.ssid,
                        iface = %device.iface,
                        ap_path = %path,
                        bssid = %ap.bssid,
                        "matched visible access point"
                    );
                    return Ok(Some((device, path, ap)));
                }
            }
        }
        tracing::debug!(ssid = %target.ssid, "no matching visible access point found");
        Ok(None)
    }

    pub(crate) fn list_access_points(&self) -> Result<Vec<AccessPoint>> {
        let mut by_ssid = BTreeMap::new();
        for device in self.wifi_devices()? {
            self.add_device_access_points(&device, &mut by_ssid)?;
        }
        let aps = sorted_access_points(by_ssid);
        tracing::debug!(
            count = aps.len(),
            "listed visible Wi-Fi networks after SSID deduplication"
        );
        Ok(aps)
    }

    fn add_device_access_points(
        &self,
        device: &WifiDevice,
        by_ssid: &mut BTreeMap<Vec<u8>, AccessPoint>,
    ) -> Result<()> {
        let active_path = self.active_access_point(device)?;
        for path in self.device_access_points(device)? {
            let active = active_path.as_ref().is_some_and(|active| *active == path);
            if let Some(ap) = self.read_visible_access_point(&device.path, &path, active) {
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

    pub(super) fn device_access_points(&self, device: &WifiDevice) -> Result<Vec<OwnedObjectPath>> {
        let wifi = self.proxy_path(&device.path, WIFI_IFACE)?;
        wifi.call("GetAccessPoints", &())
            .with_context(|| format!("GetAccessPoints for {}", device.iface))
    }

    fn read_visible_access_point(
        &self,
        device_path: &OwnedObjectPath,
        path: &OwnedObjectPath,
        active: bool,
    ) -> Option<AccessPoint> {
        match self.access_point(device_path, path, active) {
            Ok(ap) if !ap.ssid.is_empty() => Some(ap),
            Ok(_) => None,
            Err(err) => {
                eprintln!("warning: skipping access point {path}: {err:#}");
                None
            }
        }
    }

    fn access_point(
        &self,
        device_path: &OwnedObjectPath,
        path: &OwnedObjectPath,
        active: bool,
    ) -> Result<AccessPoint> {
        let ap = self.proxy_path(path, AP_IFACE)?;
        let ssid_bytes: Vec<u8> = ap
            .get_property("Ssid")
            .with_context(|| format!("read Ssid for {path}"))?;
        let flags = ap.get_property("Flags").unwrap_or(0);
        let wpa_flags = ap.get_property("WpaFlags").unwrap_or(0);
        let rsn_flags = ap.get_property("RsnFlags").unwrap_or(0);

        Ok(AccessPoint {
            ssid: display_ssid(&ssid_bytes),
            ssid_bytes,
            active,
            security: security_label(flags, wpa_flags, rsn_flags),
            strength: ap.get_property("Strength").unwrap_or(0),
            frequency: ap.get_property("Frequency").unwrap_or(0),
            bssid: ap.get_property("HwAddress").unwrap_or_default(),
            last_seen: ap.get_property("LastSeen").unwrap_or(-1),
            path: path.to_string(),
            device_path: device_path.to_string(),
            flags,
            wpa_flags,
            rsn_flags,
        })
    }
}

fn access_point_matches(
    ap: &AccessPoint,
    ssid_bytes: &[u8],
    ap_path: Option<&str>,
    bssid: Option<&str>,
) -> bool {
    if ap.ssid_bytes().as_ref() != ssid_bytes {
        return false;
    }
    let ap_path = ap_path.filter(|value| !value.is_empty());
    let bssid = bssid.filter(|value| !value.is_empty());
    match (ap_path, bssid) {
        (Some(ap_path), Some(bssid)) => ap.path == ap_path && ap.bssid.eq_ignore_ascii_case(bssid),
        (Some(ap_path), None) => ap.path == ap_path,
        (None, Some(bssid)) => ap.bssid.eq_ignore_ascii_case(bssid),
        (None, None) => true,
    }
}

fn merge_access_point(by_ssid: &mut BTreeMap<Vec<u8>, AccessPoint>, ap: AccessPoint) {
    by_ssid
        .entry(ap.ssid_bytes().into_owned())
        .and_modify(|existing| {
            if ap.active || (!existing.active && ap.strength > existing.strength) {
                *existing = ap.clone();
            }
        })
        .or_insert(ap);
}

fn sorted_access_points(by_ssid: BTreeMap<Vec<u8>, AccessPoint>) -> Vec<AccessPoint> {
    let mut aps: Vec<_> = by_ssid.into_values().collect();
    aps.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then_with(|| b.strength.cmp(&a.strength))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    aps
}

#[cfg(test)]
mod tests {
    use super::access_point_matches;
    use crate::model::AccessPoint;

    #[test]
    fn access_point_match_requires_path_and_bssid_when_both_are_supplied() {
        let ap = test_ap();

        assert!(access_point_matches(
            &ap,
            b"Example",
            Some("/ap/1"),
            Some("00:11:22:33:44:55")
        ));
        assert!(!access_point_matches(
            &ap,
            b"Example",
            Some("/ap/other"),
            Some("00:11:22:33:44:55")
        ));
        assert!(!access_point_matches(
            &ap,
            b"Example",
            Some("/ap/1"),
            Some("00:11:22:33:44:66")
        ));
    }

    fn test_ap() -> AccessPoint {
        AccessPoint {
            ssid: "Example".to_string(),
            ssid_bytes: b"Example".to_vec(),
            active: false,
            security: "WPA2/3".to_string(),
            strength: 80,
            frequency: 2412,
            bssid: "00:11:22:33:44:55".to_string(),
            last_seen: 0,
            path: "/ap/1".to_string(),
            device_path: "/device/1".to_string(),
            flags: 0,
            wpa_flags: 0,
            rsn_flags: 0,
        }
    }
}
