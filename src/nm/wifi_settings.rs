use std::collections::HashMap;

use anyhow::{Result, bail};
use zvariant::OwnedValue;

use super::{ConnectionSettings, owned_value};
use crate::model::{
    AccessPoint, NM_AP_SEC_KEY_MGMT_PSK, NM_AP_SEC_KEY_MGMT_SAE, WepKeyType, WifiConnectTarget,
};

pub(super) fn psk_wifi_connection_settings(
    ap: &AccessPoint,
    password: &str,
) -> Result<ConnectionSettings> {
    let key_mgmt = psk_key_mgmt(ap);
    if key_mgmt == "wpa-psk" {
        validate_wpa_psk(password)?;
    }
    wireless_security_settings(key_mgmt, password)
}

pub(super) fn hidden_wifi_connection_settings(
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<ConnectionSettings> {
    let mut settings =
        base_wifi_connection_settings(&target.ssid, target.ssid_bytes().as_ref(), true)?;
    if let Some(password) = password {
        let security = if let Some(wep_key_type) = wep_key_type {
            wep_security_section(password, wep_key_type)?
        } else {
            validate_wpa_psk(password)?;
            wireless_security_section("wpa-psk", password)?
        };
        settings.insert("802-11-wireless-security".to_string(), security);
    }
    Ok(settings)
}

pub(super) fn wep_wifi_connection_settings(
    password: &str,
    wep_key_type: WepKeyType,
) -> Result<ConnectionSettings> {
    Ok(security_connection_settings(wep_security_section(
        password,
        wep_key_type,
    )?))
}

fn base_wifi_connection_settings(
    ssid: &str,
    ssid_bytes: &[u8],
    hidden: bool,
) -> Result<ConnectionSettings> {
    Ok(ConnectionSettings::from([
        (
            "connection".to_string(),
            HashMap::from([
                ("id".to_string(), owned_value(ssid.to_string())?),
                (
                    "type".to_string(),
                    owned_value("802-11-wireless".to_string())?,
                ),
            ]),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([
                ("ssid".to_string(), owned_value(ssid_bytes.to_vec())?),
                (
                    "mode".to_string(),
                    owned_value("infrastructure".to_string())?,
                ),
                ("hidden".to_string(), owned_value(hidden)?),
            ]),
        ),
        (
            "ipv4".to_string(),
            HashMap::from([("method".to_string(), owned_value("auto".to_string())?)]),
        ),
        (
            "ipv6".to_string(),
            HashMap::from([("method".to_string(), owned_value("auto".to_string())?)]),
        ),
    ]))
}

fn wireless_security_settings(key_mgmt: &str, password: &str) -> Result<ConnectionSettings> {
    Ok(security_connection_settings(wireless_security_section(
        key_mgmt, password,
    )?))
}

fn security_connection_settings(section: HashMap<String, OwnedValue>) -> ConnectionSettings {
    ConnectionSettings::from([("802-11-wireless-security".to_string(), section)])
}

fn wireless_security_section(
    key_mgmt: &str,
    password: &str,
) -> Result<HashMap<String, OwnedValue>> {
    Ok(HashMap::from([
        ("key-mgmt".to_string(), owned_value(key_mgmt.to_string())?),
        ("psk".to_string(), owned_value(password.to_string())?),
    ]))
}

fn wep_security_section(
    password: &str,
    wep_key_type: WepKeyType,
) -> Result<HashMap<String, OwnedValue>> {
    validate_wep_key(password, wep_key_type)?;
    Ok(HashMap::from([
        ("key-mgmt".to_string(), owned_value("none".to_string())?),
        ("wep-key0".to_string(), owned_value(password.to_string())?),
        (
            "wep-key-type".to_string(),
            owned_value(wep_key_type.nm_value())?,
        ),
    ]))
}

pub(super) fn psk_key_mgmt(ap: &AccessPoint) -> &'static str {
    let flags = ap.wpa_flags | ap.rsn_flags;
    if flags & NM_AP_SEC_KEY_MGMT_SAE != 0 && flags & NM_AP_SEC_KEY_MGMT_PSK == 0 {
        "sae"
    } else {
        "wpa-psk"
    }
}

fn validate_wpa_psk(password: &str) -> Result<()> {
    let len = password.len();
    if (8..=63).contains(&len) || (len == 64 && password.chars().all(|ch| ch.is_ascii_hexdigit())) {
        return Ok(());
    }
    bail!("WPA-PSK password must be 8-63 characters, or 64 hexadecimal characters")
}

fn validate_wep_key(password: &str, wep_key_type: WepKeyType) -> Result<()> {
    match wep_key_type {
        WepKeyType::Key if wep_key_is_valid(password) => Ok(()),
        WepKeyType::Key => {
            bail!("WEP key must be 5 or 13 ASCII characters, or 10 or 26 hexadecimal characters")
        }
        WepKeyType::Phrase if (8..=64).contains(&password.len()) => Ok(()),
        WepKeyType::Phrase => bail!("WEP passphrase must be 8-64 characters"),
    }
}

fn wep_key_is_valid(password: &str) -> bool {
    (matches!(password.len(), 5 | 13) && password.is_ascii())
        || (matches!(password.len(), 10 | 26) && password.chars().all(|ch| ch.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::{psk_key_mgmt, psk_wifi_connection_settings, validate_wep_key, validate_wpa_psk};
    use crate::model::{AccessPoint, NM_AP_SEC_KEY_MGMT_PSK, NM_AP_SEC_KEY_MGMT_SAE, WepKeyType};

    #[test]
    fn psk_wifi_settings_include_password_and_key_mgmt() {
        let ap = test_ap(NM_AP_SEC_KEY_MGMT_PSK);
        let settings = psk_wifi_connection_settings(&ap, "secret123").expect("settings");

        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "key-mgmt"))
                .as_deref(),
            Some("wpa-psk")
        );
        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "psk"))
                .as_deref(),
            Some("secret123")
        );
    }

    #[test]
    fn sae_only_networks_use_sae_key_mgmt() {
        assert_eq!(psk_key_mgmt(&test_ap(NM_AP_SEC_KEY_MGMT_SAE)), "sae");
        assert_eq!(
            psk_key_mgmt(&test_ap(NM_AP_SEC_KEY_MGMT_SAE | NM_AP_SEC_KEY_MGMT_PSK)),
            "wpa-psk"
        );
    }

    #[test]
    fn wpa_psk_validation_matches_nmcli_shape() {
        assert!(validate_wpa_psk("12345678").is_ok());
        assert!(validate_wpa_psk(&"a".repeat(63)).is_ok());
        assert!(validate_wpa_psk(&"a".repeat(64)).is_ok());
        assert!(validate_wpa_psk("short").is_err());
        assert!(validate_wpa_psk(&"g".repeat(64)).is_err());
        assert!(validate_wpa_psk(&"a".repeat(65)).is_err());
    }

    #[test]
    fn wep_validation_matches_nmcli_shape() {
        assert!(validate_wep_key("abcde", WepKeyType::Key).is_ok());
        assert!(validate_wep_key("0011223344", WepKeyType::Key).is_ok());
        assert!(validate_wep_key("abc", WepKeyType::Key).is_err());
        assert!(validate_wep_key("éabc", WepKeyType::Key).is_err());
        assert!(validate_wep_key("not-hex-10", WepKeyType::Key).is_err());
        assert!(validate_wep_key("passphrase", WepKeyType::Phrase).is_ok());
        assert!(validate_wep_key("short", WepKeyType::Phrase).is_err());
    }

    fn test_ap(rsn_flags: u32) -> AccessPoint {
        AccessPoint {
            ssid: "Example".to_string(),
            ssid_bytes: b"Example".to_vec(),
            active: false,
            security: "WPA2/3".to_string(),
            strength: 50,
            frequency: 2412,
            bssid: "00:11:22:33:44:55".to_string(),
            last_seen: 0,
            path: "/ap".to_string(),
            device_path: "/device".to_string(),
            flags: 0,
            wpa_flags: 0,
            rsn_flags,
        }
    }

    fn setting_string(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<String> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }
}
