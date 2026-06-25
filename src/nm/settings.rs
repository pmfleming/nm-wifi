use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use zvariant::{OwnedObjectPath, OwnedValue};

use super::{
    ConnectionSettings, DEVICE_IFACE, Nm, SETTINGS_CONNECTION_IFACE, SETTINGS_IFACE, SETTINGS_PATH,
    owned_value,
};
use crate::model::{
    AccessPoint, NetworkEntry, SavedWifiConnection, WifiConnectTarget, WifiDevice, display_ssid,
    network_entries_with_profile_matches,
};

impl Nm {
    pub(super) fn saved_wifi_activation_target_for(
        &self,
        target: &WifiConnectTarget,
    ) -> Result<Option<(OwnedObjectPath, OwnedObjectPath, OwnedObjectPath)>> {
        if !target.hidden
            && let Some((device, ap_path, ap)) = self.visible_access_point_for(target)?
            && let Some(connection_path) =
                self.saved_wifi_connection_for_ap_on_device(&ap, &device)?
        {
            tracing::info!(ssid = %target.ssid, iface = %device.iface, "using saved Wi-Fi profile for selected access point");
            return Ok(Some((connection_path, device.path, ap_path)));
        }

        if target.has_specific_ap() {
            tracing::info!(ssid = %target.ssid, ap_path = ?target.ap_path, bssid = ?target.bssid, "not using generic saved-profile fallback for specific AP target");
            return Ok(None);
        }

        let target_ssid = target.ssid_bytes();
        let Some(connection_path) = self.saved_wifi_connection_for_ssid(target_ssid.as_ref())?
        else {
            return Ok(None);
        };
        let Some(device) = self.wifi_devices_for_target(target)?.into_iter().next() else {
            bail!("no matching Wi-Fi device found");
        };
        Ok(Some((connection_path, device.path, root_object_path()?)))
    }

    pub(crate) fn delete_connection(&self, path: &OwnedObjectPath) -> Result<()> {
        let connection = self.proxy_path(path, SETTINGS_CONNECTION_IFACE)?;
        tracing::info!(connection = %path, "deleting saved NetworkManager connection");
        connection
            .call::<_, _, ()>("Delete", &())
            .with_context(|| format!("Delete connection {path}"))
    }

    pub(crate) fn delete_connection_by_path(&self, path: &str) -> Result<()> {
        self.delete_connection(&OwnedObjectPath::try_from(path).context("parse connection path")?)
    }

    pub(crate) fn set_connection_autoconnect_by_path(
        &self,
        path: &str,
        autoconnect: bool,
    ) -> Result<()> {
        let path = OwnedObjectPath::try_from(path).context("parse connection path")?;
        let mut settings = self.connection_settings(&path)?;
        let connection = settings.entry("connection".to_string()).or_default();
        connection.insert("autoconnect".to_string(), owned_value(autoconnect)?);
        let proxy = self.proxy_path(&path, SETTINGS_CONNECTION_IFACE)?;
        tracing::info!(connection = %path, autoconnect, "updating saved NetworkManager connection autoconnect");
        proxy
            .call::<_, _, ()>("Update", &(settings,))
            .with_context(|| format!("Update autoconnect for {path}"))
    }

    pub(crate) fn saved_wifi_connections(&self) -> Result<Vec<SavedWifiConnection>> {
        let mut connections = Vec::new();
        for path in self.saved_connections()? {
            let settings = self.connection_settings(&path)?;
            if let Some(connection) = saved_wifi_connection_from_settings(&path, &settings) {
                connections.push(connection);
            }
        }
        connections.sort_by_key(|connection| connection.id.to_lowercase());
        Ok(connections)
    }

    pub(crate) fn network_entries_for_access_points(
        &self,
        access_points: Vec<AccessPoint>,
    ) -> Result<Vec<NetworkEntry>> {
        let profile_matches = self.compatible_profile_matches_by_ap_path(&access_points)?;
        Ok(network_entries_with_profile_matches(
            access_points,
            &profile_matches,
        ))
    }

    fn compatible_profile_matches_by_ap_path(
        &self,
        access_points: &[AccessPoint],
    ) -> Result<std::collections::BTreeMap<String, Vec<SavedWifiConnection>>> {
        let profiles_by_path = self
            .saved_wifi_connections()?
            .into_iter()
            .map(|profile| (profile.path.clone(), profile))
            .collect::<HashMap<_, _>>();
        let mut matches = std::collections::BTreeMap::new();
        for access_point in access_points {
            self.add_compatible_profile_matches(access_point, &profiles_by_path, &mut matches);
        }
        Ok(matches)
    }

    fn add_compatible_profile_matches(
        &self,
        access_point: &AccessPoint,
        profiles_by_path: &HashMap<String, SavedWifiConnection>,
        matches: &mut std::collections::BTreeMap<String, Vec<SavedWifiConnection>>,
    ) {
        let Ok(device_path) = OwnedObjectPath::try_from(access_point.device_path.as_str()) else {
            tracing::warn!(device_path = %access_point.device_path, ap_path = %access_point.path, "skipping AP profile compatibility check with invalid device path");
            return;
        };
        let device = WifiDevice {
            path: device_path,
            iface: access_point.device_iface.clone(),
        };
        let available = match self.available_connections(&device) {
            Ok(available) => available,
            Err(err) => {
                tracing::warn!(iface = %device.iface, error = %format_args!("{err:#}"), "could not read AvailableConnections for AP profile compatibility");
                return;
            }
        };
        for path in available {
            match self.connection_matches_access_point(&path, access_point) {
                Ok(true) => {
                    if let Some(profile) = profiles_by_path.get(&path.to_string()) {
                        matches
                            .entry(access_point.path.clone())
                            .or_default()
                            .push(profile.clone());
                    }
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(connection = %path, error = %format_args!("{err:#}"), "could not check saved profile compatibility")
                }
            }
        }
    }

    fn saved_wifi_connection_for_ap_on_device(
        &self,
        ap: &AccessPoint,
        device: &WifiDevice,
    ) -> Result<Option<OwnedObjectPath>> {
        for path in self.available_connections(device)? {
            if self.connection_matches_access_point(&path, ap)? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn saved_wifi_connection_for_ssid(&self, ssid_bytes: &[u8]) -> Result<Option<OwnedObjectPath>> {
        for path in self.saved_connections()? {
            if self.connection_matches_ssid(&path, ssid_bytes)? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn connection_matches_ssid(&self, path: &OwnedObjectPath, ssid_bytes: &[u8]) -> Result<bool> {
        let settings = self.connection_settings(path)?;
        Ok(settings_match_wifi_ssid(&settings, ssid_bytes))
    }

    fn connection_matches_access_point(
        &self,
        path: &OwnedObjectPath,
        ap: &AccessPoint,
    ) -> Result<bool> {
        let settings = self.connection_settings(path)?;
        Ok(settings_match_access_point(&settings, ap))
    }

    fn saved_connections(&self) -> Result<Vec<OwnedObjectPath>> {
        let settings = self.proxy(SETTINGS_PATH, SETTINGS_IFACE)?;
        settings
            .call("ListConnections", &())
            .context("ListConnections")
    }

    fn connection_settings(&self, path: &OwnedObjectPath) -> Result<ConnectionSettings> {
        let connection = self.proxy_path(path, SETTINGS_CONNECTION_IFACE)?;
        connection
            .call("GetSettings", &())
            .with_context(|| format!("GetSettings for {path}"))
    }

    fn available_connections(&self, device: &WifiDevice) -> Result<Vec<OwnedObjectPath>> {
        let device_proxy = self.proxy_path(&device.path, DEVICE_IFACE)?;
        device_proxy
            .get_property("AvailableConnections")
            .with_context(|| format!("read AvailableConnections for {}", device.iface))
    }
}

fn saved_wifi_connection_from_settings(
    path: &OwnedObjectPath,
    settings: &ConnectionSettings,
) -> Option<SavedWifiConnection> {
    let connection = settings.get("connection")?;
    let wireless = settings.get("802-11-wireless")?;
    if connection
        .get("type")
        .and_then(setting_value_string)
        .is_some_and(|connection_type| connection_type != "802-11-wireless")
    {
        return None;
    }
    let id = setting_string(connection, "id").unwrap_or_else(|| path.to_string());
    let ssid_bytes = wireless
        .get("ssid")
        .and_then(setting_bytes)
        .unwrap_or_default();
    let ssid = display_ssid(&ssid_bytes);
    let autoconnect = connection
        .get("autoconnect")
        .and_then(setting_bool)
        .unwrap_or(true);
    Some(SavedWifiConnection {
        path: path.to_string(),
        id,
        ssid,
        ssid_bytes,
        autoconnect,
    })
}

fn settings_match_wifi_ssid(settings: &ConnectionSettings, ssid_bytes: &[u8]) -> bool {
    let Some(wireless) = wifi_settings_section(settings) else {
        return false;
    };
    wireless
        .get("ssid")
        .and_then(setting_bytes)
        .is_some_and(|saved_ssid| ssid_bytes_match(&saved_ssid, ssid_bytes))
}

fn settings_match_access_point(settings: &ConnectionSettings, ap: &AccessPoint) -> bool {
    let Some(wireless) = wifi_settings_section(settings) else {
        return false;
    };
    if !wireless
        .get("ssid")
        .and_then(setting_bytes)
        .is_some_and(|saved_ssid| ssid_bytes_match(&saved_ssid, ap.ssid_bytes().as_ref()))
    {
        return false;
    }
    wireless
        .get("bssid")
        .and_then(setting_bytes)
        .is_none_or(|saved_bssid| bssid_bytes_match(&saved_bssid, &ap.bssid))
}

fn wifi_settings_section(settings: &ConnectionSettings) -> Option<&HashMap<String, OwnedValue>> {
    let wireless = settings.get("802-11-wireless")?;
    if settings
        .get("connection")
        .and_then(|connection| setting_string(connection, "type"))
        .is_some_and(|connection_type| connection_type != "802-11-wireless")
    {
        return None;
    }
    Some(wireless)
}

fn setting_string(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    settings.get(key).and_then(setting_value_string)
}

fn setting_value_string(value: &OwnedValue) -> Option<String> {
    value.try_clone().ok()?.try_into().ok()
}

fn setting_bool(value: &OwnedValue) -> Option<bool> {
    value.try_clone().ok()?.try_into().ok()
}

fn setting_bytes(value: &OwnedValue) -> Option<Vec<u8>> {
    value.try_clone().ok()?.try_into().ok()
}

fn ssid_bytes_match(saved_ssid: &[u8], ssid_bytes: &[u8]) -> bool {
    saved_ssid == ssid_bytes
}

fn bssid_bytes_match(saved_bssid: &[u8], ap_bssid: &str) -> bool {
    parse_bssid(ap_bssid).is_some_and(|ap_bssid| saved_bssid == ap_bssid)
}

fn parse_bssid(value: &str) -> Option<Vec<u8>> {
    let bytes: Option<Vec<_>> = value
        .split([':', '-'])
        .map(|part| u8::from_str_radix(part, 16).ok())
        .collect();
    bytes.filter(|bytes| bytes.len() == 6)
}

fn root_object_path() -> Result<OwnedObjectPath> {
    OwnedObjectPath::try_from("/").context("create root object path")
}

#[cfg(test)]
mod tests {
    use super::{ConnectionSettings, settings_match_wifi_ssid, ssid_bytes_match};
    use std::collections::HashMap;
    use zvariant::{OwnedValue, Value};

    #[test]
    fn ssid_bytes_match_exact_bytes() {
        assert!(ssid_bytes_match(b"Example", b"Example"));
        assert!(ssid_bytes_match(&[0xff], &[0xff]));
        assert!(!ssid_bytes_match(&[0xff], "�".as_bytes()));
    }

    #[test]
    fn settings_match_wireless_ssid() {
        let settings = wifi_settings("Example", "802-11-wireless");

        assert!(settings_match_wifi_ssid(&settings, b"Example"));
        assert!(!settings_match_wifi_ssid(&settings, b"Other"));
    }

    #[test]
    fn settings_reject_non_wireless_profiles() {
        let settings = wifi_settings("Example", "ethernet");

        assert!(!settings_match_wifi_ssid(&settings, b"Example"));
    }

    fn wifi_settings(ssid: &str, connection_type: &str) -> ConnectionSettings {
        let mut settings = ConnectionSettings::new();
        settings.insert(
            "connection".to_string(),
            HashMap::from([(
                "type".to_string(),
                owned_value(Value::new(connection_type.to_string())),
            )]),
        );
        settings.insert(
            "802-11-wireless".to_string(),
            HashMap::from([(
                "ssid".to_string(),
                owned_value(Value::new(ssid.as_bytes().to_vec())),
            )]),
        );
        settings
    }

    fn owned_value(value: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(value).expect("owned value")
    }
}
