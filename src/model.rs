use std::borrow::Cow;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zvariant::OwnedObjectPath;

pub(crate) const NM_AP_FLAGS_PRIVACY: u32 = 0x1;
pub(crate) const NM_AP_SEC_KEY_MGMT_PSK: u32 = 0x0000_0100;
pub(crate) const NM_AP_SEC_KEY_MGMT_802_1X: u32 = 0x0000_0200;
pub(crate) const NM_AP_SEC_KEY_MGMT_SAE: u32 = 0x0000_0400;
pub(crate) const NM_AP_SEC_KEY_MGMT_OWE: u32 = 0x0000_0800;
pub(crate) const NM_AP_SEC_KEY_MGMT_OWE_TM: u32 = 0x0000_1000;
pub(crate) const NM_AP_SEC_KEY_MGMT_EAP_SUITE_B_192: u32 = 0x0000_2000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanStreamOptions {
    pub(crate) timeout: Duration,
    pub(crate) retries: u32,
    pub(crate) cache: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConnectResult {
    pub(crate) status: &'static str,
    pub(crate) ssid: String,
    pub(crate) message: String,
    pub(crate) connectivity: Option<ConnectivityStatus>,
    pub(crate) suggest_open_portal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConnectivityStatus {
    pub(crate) code: u32,
    pub(crate) state: &'static str,
    pub(crate) captive_portal: bool,
    pub(crate) full: bool,
}

impl ConnectivityStatus {
    pub(crate) fn from_nm_code(code: u32) -> Self {
        let state = match code {
            1 => "none",
            2 => "portal",
            3 => "limited",
            4 => "full",
            _ => "unknown",
        };
        Self {
            code,
            state,
            captive_portal: matches!(code, 2 | 3),
            full: code == 4,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum WepKeyType {
    Key,
    Phrase,
}

impl WepKeyType {
    pub(crate) fn nm_value(self) -> u32 {
        match self {
            Self::Key => 1,
            Self::Phrase => 2,
        }
    }

    pub(crate) fn nmcli_value(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Phrase => "phrase",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WifiDevice {
    pub(crate) path: OwnedObjectPath,
    pub(crate) iface: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct SavedWifiConnection {
    pub(crate) path: String,
    pub(crate) id: String,
    /// Human-readable display form of the SSID. This may be lossy for non-UTF-8 SSIDs.
    pub(crate) ssid: String,
    /// Exact SSID bytes used for identity/matching.
    pub(crate) ssid_bytes: Vec<u8>,
    pub(crate) autoconnect: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct WifiConnectTarget {
    /// Human-readable display form of the SSID. This may be lossy for non-UTF-8 SSIDs.
    pub(crate) ssid: String,
    /// Exact SSID bytes used for identity/matching. Empty only for legacy cache/action records.
    #[serde(default)]
    pub(crate) ssid_bytes: Vec<u8>,
    #[serde(alias = "path")]
    pub(crate) ap_path: Option<String>,
    pub(crate) bssid: Option<String>,
    #[serde(default)]
    pub(crate) hidden: bool,
    #[serde(default)]
    pub(crate) security: Option<String>,
}

impl WifiConnectTarget {
    pub(crate) fn ssid_bytes(&self) -> Cow<'_, [u8]> {
        if self.ssid_bytes.is_empty() {
            Cow::Borrowed(self.ssid.as_bytes())
        } else {
            Cow::Borrowed(&self.ssid_bytes)
        }
    }

    pub(crate) fn has_specific_ap(&self) -> bool {
        self.ap_path
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || self.bssid.as_deref().is_some_and(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct NetworkCapabilities {
    pub(crate) can_connect: bool,
    pub(crate) needs_password: bool,
    pub(crate) can_forget: bool,
    pub(crate) can_toggle_autoconnect: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct NetworkEntry {
    #[serde(flatten)]
    pub(crate) access_point: AccessPoint,
    pub(crate) primary_profile: Option<SavedWifiConnection>,
    pub(crate) profiles: Vec<SavedWifiConnection>,
    pub(crate) capabilities: NetworkCapabilities,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct AccessPoint {
    /// Human-readable display form of the SSID. This may be lossy for non-UTF-8 SSIDs.
    pub(crate) ssid: String,
    /// Exact SSID bytes used for identity/matching. Empty only for legacy cache records.
    #[serde(default)]
    pub(crate) ssid_bytes: Vec<u8>,
    pub(crate) active: bool,
    pub(crate) security: String,
    pub(crate) strength: u8,
    pub(crate) frequency: u32,
    pub(crate) bssid: String,
    pub(crate) last_seen: i32,
    #[serde(default)]
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) device_path: String,
    #[serde(default)]
    pub(crate) flags: u32,
    #[serde(default)]
    pub(crate) wpa_flags: u32,
    #[serde(default)]
    pub(crate) rsn_flags: u32,
}

impl AccessPoint {
    pub(crate) fn ssid_bytes(&self) -> Cow<'_, [u8]> {
        if self.ssid_bytes.is_empty() {
            Cow::Borrowed(self.ssid.as_bytes())
        } else {
            Cow::Borrowed(&self.ssid_bytes)
        }
    }
}

pub(crate) fn network_entries(
    access_points: Vec<AccessPoint>,
    profiles: &[SavedWifiConnection],
) -> Vec<NetworkEntry> {
    access_points
        .into_iter()
        .map(|access_point| network_entry(access_point, profiles))
        .collect()
}

fn network_entry(access_point: AccessPoint, profiles: &[SavedWifiConnection]) -> NetworkEntry {
    let profiles = profiles_for_access_point(&access_point, profiles);
    let primary_profile = profiles.first().cloned();
    let needs_password = primary_profile.is_none()
        && !ap_is_passwordless(
            access_point.flags,
            access_point.wpa_flags,
            access_point.rsn_flags,
        )
        && (ap_supports_psk(access_point.wpa_flags, access_point.rsn_flags)
            || ap_uses_wep(
                access_point.flags,
                access_point.wpa_flags,
                access_point.rsn_flags,
            ));
    let has_identity = !access_point.ssid_bytes().is_empty();
    let has_profile = primary_profile.is_some();
    NetworkEntry {
        access_point,
        primary_profile,
        capabilities: NetworkCapabilities {
            can_connect: has_identity && (!needs_password || has_profile),
            needs_password,
            can_forget: has_profile,
            can_toggle_autoconnect: has_profile,
        },
        profiles,
    }
}

fn profiles_for_access_point(
    access_point: &AccessPoint,
    profiles: &[SavedWifiConnection],
) -> Vec<SavedWifiConnection> {
    let ap_ssid = access_point.ssid_bytes();
    profiles
        .iter()
        .filter(|profile| ssid_matches(ap_ssid.as_ref(), &profile.ssid_bytes))
        .cloned()
        .collect()
}

fn ssid_matches(left: &[u8], right: &[u8]) -> bool {
    !left.is_empty() && !right.is_empty() && left == right
}

#[derive(Debug)]
pub(crate) enum ScanEvent {
    WatcherReady,
    WatcherWarning(String),
    AccessPointsChanged,
    LastScanChanged { device_path: String, value: i64 },
}

pub(crate) fn display_ssid(ssid_bytes: &[u8]) -> String {
    String::from_utf8_lossy(ssid_bytes).into_owned()
}

pub(crate) fn security_label(flags: u32, wpa_flags: u32, rsn_flags: u32) -> String {
    if ap_is_passwordless(flags, wpa_flags, rsn_flags) {
        if has_owe(wpa_flags | rsn_flags) {
            "OWE".to_string()
        } else {
            "--".to_string()
        }
    } else if rsn_flags != 0 {
        "WPA2/3".to_string()
    } else if wpa_flags != 0 {
        "WPA".to_string()
    } else {
        "WEP".to_string()
    }
}

pub(crate) fn ap_is_passwordless(flags: u32, wpa_flags: u32, rsn_flags: u32) -> bool {
    let privacy = flags & NM_AP_FLAGS_PRIVACY != 0;
    !privacy && flags_are_passwordless(wpa_flags) && flags_are_passwordless(rsn_flags)
}

pub(crate) fn ap_supports_psk(wpa_flags: u32, rsn_flags: u32) -> bool {
    let flags = wpa_flags | rsn_flags;
    flags & (NM_AP_SEC_KEY_MGMT_PSK | NM_AP_SEC_KEY_MGMT_SAE) != 0
}

pub(crate) fn ap_uses_wep(flags: u32, wpa_flags: u32, rsn_flags: u32) -> bool {
    flags & NM_AP_FLAGS_PRIVACY != 0 && wpa_flags == 0 && rsn_flags == 0
}

fn flags_are_passwordless(flags: u32) -> bool {
    let secret_key_mgmt = NM_AP_SEC_KEY_MGMT_PSK
        | NM_AP_SEC_KEY_MGMT_802_1X
        | NM_AP_SEC_KEY_MGMT_SAE
        | NM_AP_SEC_KEY_MGMT_EAP_SUITE_B_192;
    flags & secret_key_mgmt == 0 && (flags == 0 || has_owe(flags))
}

fn has_owe(flags: u32) -> bool {
    flags & (NM_AP_SEC_KEY_MGMT_OWE | NM_AP_SEC_KEY_MGMT_OWE_TM) != 0
}

pub(crate) fn retry_delay(attempts: u32) -> Duration {
    Duration::from_secs(2_u64.pow(attempts.saturating_sub(1).min(3)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        ConnectivityStatus, NM_AP_FLAGS_PRIVACY, NM_AP_SEC_KEY_MGMT_OWE, NM_AP_SEC_KEY_MGMT_PSK,
        NM_AP_SEC_KEY_MGMT_SAE, WifiConnectTarget, ap_is_passwordless, ap_supports_psk,
        ap_uses_wep, retry_delay, security_label,
    };

    #[test]
    fn retry_delay_uses_bounded_exponential_backoff() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(3), Duration::from_secs(4));
        assert_eq!(retry_delay(4), Duration::from_secs(8));
        assert_eq!(retry_delay(99), Duration::from_secs(8));
    }

    #[test]
    fn security_label_identifies_open_networks() {
        assert_eq!(security_label(0, 0, 0), "--");
    }

    #[test]
    fn security_label_prefers_rsn_over_wpa() {
        assert_eq!(security_label(NM_AP_FLAGS_PRIVACY, 1, 1), "WPA2/3");
        assert_eq!(security_label(NM_AP_FLAGS_PRIVACY, 1, 0), "WPA");
        assert_eq!(security_label(NM_AP_FLAGS_PRIVACY, 0, 0), "WEP");
    }

    #[test]
    fn owe_is_passwordless_but_psk_is_not() {
        assert!(ap_is_passwordless(0, 0, NM_AP_SEC_KEY_MGMT_OWE));
        assert_eq!(security_label(0, 0, NM_AP_SEC_KEY_MGMT_OWE), "OWE");
        assert!(!ap_is_passwordless(0, 0, NM_AP_SEC_KEY_MGMT_PSK));
    }

    #[test]
    fn psk_support_includes_sae() {
        assert!(ap_supports_psk(NM_AP_SEC_KEY_MGMT_PSK, 0));
        assert!(ap_supports_psk(0, NM_AP_SEC_KEY_MGMT_SAE));
        assert!(!ap_supports_psk(0, NM_AP_SEC_KEY_MGMT_OWE));
    }

    #[test]
    fn wep_detection_requires_privacy_without_wpa_or_rsn() {
        assert!(ap_uses_wep(NM_AP_FLAGS_PRIVACY, 0, 0));
        assert!(!ap_uses_wep(0, 0, 0));
        assert!(!ap_uses_wep(NM_AP_FLAGS_PRIVACY, NM_AP_SEC_KEY_MGMT_PSK, 0));
    }

    #[test]
    fn connectivity_status_maps_networkmanager_codes() {
        let portal = ConnectivityStatus::from_nm_code(2);
        assert_eq!(portal.state, "portal");
        assert!(portal.captive_portal);
        assert!(!portal.full);

        let full = ConnectivityStatus::from_nm_code(4);
        assert_eq!(full.state, "full");
        assert!(!full.captive_portal);
        assert!(full.full);
    }

    #[test]
    fn connect_target_accepts_network_entry_path_alias() {
        let target: WifiConnectTarget = serde_json::from_str(
            r#"{
                "ssid": "Cafe",
                "ssid_bytes": [67, 97, 102, 101],
                "path": "/org/freedesktop/NetworkManager/AccessPoint/1",
                "bssid": "00:11:22:33:44:55"
            }"#,
        )
        .expect("target JSON");

        assert_eq!(target.ssid, "Cafe");
        assert_eq!(target.ssid_bytes, b"Cafe");
        assert_eq!(
            target.ap_path.as_deref(),
            Some("/org/freedesktop/NetworkManager/AccessPoint/1")
        );
        assert_eq!(target.bssid.as_deref(), Some("00:11:22:33:44:55"));
        assert!(!target.hidden);
    }
}
