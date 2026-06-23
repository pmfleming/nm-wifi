use std::time::{Duration, Instant};

use serde::Serialize;
use zvariant::OwnedObjectPath;

pub(crate) const NM_AP_FLAGS_PRIVACY: u32 = 0x1;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanStreamOptions {
    pub(crate) timeout: Duration,
    pub(crate) retries: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct WifiDevice {
    pub(crate) path: OwnedObjectPath,
    pub(crate) iface: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AccessPoint {
    pub(crate) ssid: String,
    pub(crate) active: bool,
    pub(crate) security: String,
    pub(crate) strength: u8,
    pub(crate) frequency: u32,
    pub(crate) bssid: String,
    pub(crate) last_seen: i32,
}

#[derive(Debug)]
pub(crate) struct DeviceScanState {
    pub(crate) device: WifiDevice,
    pub(crate) before: i64,
    pub(crate) completed: bool,
    pub(crate) attempts: u32,
    pub(crate) next_retry: Option<Instant>,
}

#[derive(Debug)]
pub(crate) enum ScanEvent {
    WatcherReady,
    WatcherWarning(String),
    AccessPointsChanged,
    LastScanChanged { device_path: String, value: i64 },
}

pub(crate) fn security_label(flags: u32, wpa_flags: u32, rsn_flags: u32) -> String {
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

pub(crate) fn retry_delay(attempts: u32) -> Duration {
    Duration::from_secs(2_u64.pow(attempts.saturating_sub(1).min(3)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{NM_AP_FLAGS_PRIVACY, retry_delay, security_label};

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
}
