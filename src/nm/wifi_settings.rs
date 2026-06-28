use std::collections::HashMap;

use anyhow::{Result, bail};
use zvariant::OwnedValue;

use super::{ConnectionSettings, owned_value};
use crate::model::{
    AccessPoint, EnterpriseAuth, NM_AP_SEC_KEY_MGMT_PSK, NM_AP_SEC_KEY_MGMT_SAE, TargetIpAddress,
    TargetIpRoute, TargetIpSettings, TargetProfileSettings, WepKeyType, WifiConnectTarget,
    ap_uses_owe, enterprise_key_mgmt,
};

pub(super) fn psk_wifi_connection_settings(
    ap: &AccessPoint,
    password: &str,
) -> Result<ConnectionSettings> {
    let key_mgmt = psk_key_mgmt(ap);
    if key_mgmt == "wpa-psk" {
        validate_wpa_psk(password)?;
    }
    Ok(security_connection_settings(wireless_security_section(
        key_mgmt, password,
    )?))
}

pub(super) fn owe_wifi_connection_settings() -> Result<ConnectionSettings> {
    Ok(security_connection_settings(HashMap::from([(
        "key-mgmt".to_string(),
        owned_value("owe".to_string())?,
    )])))
}

pub(super) fn hidden_wifi_connection_settings(
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<ConnectionSettings> {
    let mut settings =
        base_wifi_connection_settings(&target.ssid, target.ssid_bytes().as_ref(), true)?;
    if let Some(security) = security_settings_for_target_hint(target, password, wep_key_type)? {
        settings.extend(security);
    }
    apply_target_profile_settings(&mut settings, target)?;
    Ok(settings)
}

pub(super) fn cloned_wifi_connection_settings(
    mut settings: ConnectionSettings,
    target: &WifiConnectTarget,
    ap: &AccessPoint,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<ConnectionSettings> {
    sanitize_cloned_settings(&mut settings)?;
    ensure_wireless_settings(&mut settings, target, false)?;
    if let Some(security) = security_settings_for_visible_ap(ap, target, password, wep_key_type)? {
        settings.extend(security);
    }
    apply_target_profile_settings(&mut settings, target)?;
    Ok(settings)
}

pub(super) fn enterprise_wifi_connection_settings(
    ap: &AccessPoint,
    enterprise: &EnterpriseAuth,
    password: Option<&str>,
) -> Result<ConnectionSettings> {
    enterprise_wifi_connection_settings_with_key_mgmt(
        enterprise,
        password,
        enterprise_key_mgmt(ap.wpa_flags, ap.rsn_flags),
    )
}

fn security_settings_for_visible_ap(
    ap: &AccessPoint,
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<Option<ConnectionSettings>> {
    if let Some(enterprise) = &target.enterprise {
        return enterprise_wifi_connection_settings(ap, enterprise, password).map(Some);
    }
    let Some(password) = password else {
        return Ok(None);
    };
    if crate::model::ap_uses_wep(ap.flags, ap.wpa_flags, ap.rsn_flags) {
        return wep_wifi_connection_settings(password, wep_key_type.unwrap_or(WepKeyType::Key))
            .map(Some);
    }
    if crate::model::ap_supports_psk(ap.wpa_flags, ap.rsn_flags) {
        return psk_wifi_connection_settings(ap, password).map(Some);
    }
    if ap_uses_owe(ap.wpa_flags, ap.rsn_flags) {
        return owe_wifi_connection_settings().map(Some);
    }
    Ok(None)
}

fn security_settings_for_target_hint(
    target: &WifiConnectTarget,
    password: Option<&str>,
    wep_key_type: Option<WepKeyType>,
) -> Result<Option<ConnectionSettings>> {
    let key_mgmt = target
        .key_mgmt
        .as_deref()
        .or_else(|| {
            target
                .enterprise
                .as_ref()
                .and_then(|auth| auth.key_mgmt.as_deref())
        })
        .map(normalized_key_mgmt);
    if let Some(enterprise) = &target.enterprise {
        let key_mgmt = key_mgmt.as_deref().unwrap_or("wpa-eap");
        return enterprise_wifi_connection_settings_with_key_mgmt(enterprise, password, key_mgmt)
            .map(Some);
    }

    match key_mgmt.as_deref() {
        None => {
            let Some(password) = password else {
                return Ok(None);
            };
            if let Some(wep_key_type) = wep_key_type {
                wep_wifi_connection_settings(password, wep_key_type).map(Some)
            } else {
                validate_wpa_psk(password)?;
                Ok(Some(security_connection_settings(
                    wireless_security_section("wpa-psk", password)?,
                )))
            }
        }
        Some("open" | "none" | "--") => Ok(None),
        Some("owe") => owe_wifi_connection_settings().map(Some),
        Some("wep") => {
            let Some(password) = password else {
                bail!("hidden WEP network requires a password/key")
            };
            wep_wifi_connection_settings(password, wep_key_type.unwrap_or(WepKeyType::Key))
                .map(Some)
        }
        Some("sae" | "wpa-psk") => {
            let Some(password) = password else {
                bail!("hidden WPA/SAE network requires a password")
            };
            validate_wpa_psk(password)?;
            Ok(Some(security_connection_settings(
                wireless_security_section(key_mgmt.as_deref().unwrap_or("wpa-psk"), password)?,
            )))
        }
        Some("wpa-eap" | "wpa-eap-suite-b-192") => {
            bail!("hidden enterprise network requires an enterprise credential object")
        }
        Some(other) => bail!("unsupported hidden key management '{other}'"),
    }
}

fn enterprise_wifi_connection_settings_with_key_mgmt(
    enterprise: &EnterpriseAuth,
    password: Option<&str>,
    key_mgmt: &str,
) -> Result<ConnectionSettings> {
    let mut security = HashMap::new();
    security.insert("key-mgmt".to_string(), owned_value(key_mgmt.to_string())?);

    let mut dot1x = HashMap::new();
    let eap = if enterprise.eap.is_empty() {
        vec!["peap".to_string()]
    } else {
        enterprise.eap.clone()
    };
    dot1x.insert("eap".to_string(), owned_value(eap)?);
    insert_required_string(&mut dot1x, "identity", enterprise.identity.as_deref())?;
    insert_optional_strings(
        &mut dot1x,
        &[
            (
                "anonymous-identity",
                enterprise.anonymous_identity.as_deref(),
            ),
            ("password", enterprise.password.as_deref().or(password)),
            ("phase2-auth", enterprise.phase2_auth.as_deref()),
            ("ca-cert", enterprise.ca_cert.as_deref()),
            ("ca-path", enterprise.ca_path.as_deref()),
            (
                "domain-suffix-match",
                enterprise.domain_suffix_match.as_deref(),
            ),
            ("subject-match", enterprise.subject_match.as_deref()),
            ("openssl-ciphers", enterprise.openssl_ciphers.as_deref()),
            ("phase1-peapver", enterprise.phase1_peapver.as_deref()),
            ("phase1-peaplabel", enterprise.phase1_peaplabel.as_deref()),
            (
                "phase1-fast-provisioning",
                enterprise.phase1_fast_provisioning.as_deref(),
            ),
        ],
    )?;
    if !enterprise.altsubject_matches.is_empty() {
        dot1x.insert(
            "altsubject-matches".to_string(),
            owned_value(enterprise.altsubject_matches.clone())?,
        );
    }
    insert_optional_strings(
        &mut dot1x,
        &[
            ("client-cert", enterprise.client_cert.as_deref()),
            ("private-key", enterprise.private_key.as_deref()),
            (
                "private-key-password",
                enterprise.private_key_password.as_deref(),
            ),
            ("pin", enterprise.pin.as_deref()),
            ("pac-file", enterprise.pac_file.as_deref()),
        ],
    )?;
    if let Some(system_ca_certs) = enterprise.system_ca_certs {
        dot1x.insert("system-ca-certs".to_string(), owned_value(system_ca_certs)?);
    }
    insert_optional_u32s(
        &mut dot1x,
        &[
            ("password-flags", enterprise.password_flags),
            (
                "private-key-password-flags",
                enterprise.private_key_password_flags,
            ),
            ("pin-flags", enterprise.pin_flags),
        ],
    )?;

    Ok(ConnectionSettings::from([
        ("802-11-wireless-security".to_string(), security),
        ("802-1x".to_string(), dot1x),
    ]))
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

fn sanitize_cloned_settings(settings: &mut ConnectionSettings) -> Result<()> {
    let connection = settings.entry("connection".to_string()).or_default();
    connection.remove("uuid");
    connection.remove("timestamp");
    connection.insert(
        "type".to_string(),
        owned_value("802-11-wireless".to_string())?,
    );
    Ok(())
}

fn ensure_wireless_settings(
    settings: &mut ConnectionSettings,
    target: &WifiConnectTarget,
    hidden: bool,
) -> Result<()> {
    let connection = settings.entry("connection".to_string()).or_default();
    connection
        .entry("id".to_string())
        .or_insert(owned_value(target.ssid.clone())?);
    connection
        .entry("type".to_string())
        .or_insert(owned_value("802-11-wireless".to_string())?);

    let wireless = settings.entry("802-11-wireless".to_string()).or_default();
    wireless.insert(
        "ssid".to_string(),
        owned_value(target.ssid_bytes().to_vec())?,
    );
    wireless.insert(
        "mode".to_string(),
        owned_value("infrastructure".to_string())?,
    );
    wireless.insert("hidden".to_string(), owned_value(hidden)?);
    Ok(())
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

fn security_connection_settings(section: HashMap<String, OwnedValue>) -> ConnectionSettings {
    ConnectionSettings::from([("802-11-wireless-security".to_string(), section)])
}

pub(super) fn apply_target_profile_settings(
    settings: &mut ConnectionSettings,
    target: &WifiConnectTarget,
) -> Result<()> {
    apply_profile_settings(settings, &target.profile)
}

fn apply_profile_settings(
    settings: &mut ConnectionSettings,
    profile: &TargetProfileSettings,
) -> Result<()> {
    let connection = settings.entry("connection".to_string()).or_default();
    if let Some(autoconnect) = profile.autoconnect {
        connection.insert("autoconnect".to_string(), owned_value(autoconnect)?);
    }
    if let Some(priority) = profile.autoconnect_priority {
        connection.insert("autoconnect-priority".to_string(), owned_value(priority)?);
    }
    if let Some(metered) = profile.metered.as_deref().filter(|value| !value.is_empty()) {
        connection.insert("metered".to_string(), owned_value(metered.to_string())?);
    }
    if let Some(cloned_mac) = profile
        .cloned_mac_address
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        settings
            .entry("802-11-wireless".to_string())
            .or_default()
            .insert(
                "cloned-mac-address".to_string(),
                owned_value(cloned_mac.to_string())?,
            );
    }
    if let Some(enabled) = profile.send_hostname {
        apply_send_hostname(settings, "ipv4", enabled)?;
        apply_send_hostname(settings, "ipv6", enabled)?;
    }
    if let Some(ipv4) = &profile.ipv4 {
        apply_ip_settings(settings, "ipv4", ipv4)?;
    }
    if let Some(ipv6) = &profile.ipv6 {
        apply_ip_settings(settings, "ipv6", ipv6)?;
    }
    Ok(())
}

fn apply_ip_settings(
    settings: &mut ConnectionSettings,
    section: &str,
    ip: &TargetIpSettings,
) -> Result<()> {
    let values = settings.entry(section.to_string()).or_default();
    if let Some(method) = ip
        .method
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| (!ip.addresses.is_empty()).then_some("manual".to_string()))
    {
        values.insert("method".to_string(), owned_value(method)?);
    }
    if !ip.addresses.is_empty() {
        values.insert(
            "address-data".to_string(),
            owned_value(address_data(&ip.addresses)?)?,
        );
    }
    if let Some(gateway) = ip.gateway.as_deref().filter(|value| !value.is_empty()) {
        values.insert("gateway".to_string(), owned_value(gateway.to_string())?);
    }
    if !ip.dns.is_empty() {
        values.insert("dns-data".to_string(), owned_value(ip.dns.clone())?);
    }
    if !ip.routes.is_empty() {
        values.insert(
            "route-data".to_string(),
            owned_value(route_data(&ip.routes)?)?,
        );
    }
    if let Some(route_metric) = ip.route_metric {
        values.insert("route-metric".to_string(), owned_value(route_metric)?);
    }
    if let Some(ignore_auto_dns) = ip.ignore_auto_dns {
        values.insert("ignore-auto-dns".to_string(), owned_value(ignore_auto_dns)?);
    }
    if !ip.dns_search.is_empty() {
        values.insert(
            "dns-search".to_string(),
            owned_value(ip.dns_search.clone())?,
        );
    }
    Ok(())
}

fn address_data(addresses: &[TargetIpAddress]) -> Result<Vec<HashMap<String, OwnedValue>>> {
    addresses
        .iter()
        .map(|address| {
            Ok(HashMap::from([
                ("address".to_string(), owned_value(address.address.clone())?),
                ("prefix".to_string(), owned_value(address.prefix)?),
            ]))
        })
        .collect()
}

fn route_data(routes: &[TargetIpRoute]) -> Result<Vec<HashMap<String, OwnedValue>>> {
    routes
        .iter()
        .map(|route| {
            let mut entry = HashMap::from([
                ("dest".to_string(), owned_value(route.dest.clone())?),
                ("prefix".to_string(), owned_value(route.prefix)?),
            ]);
            if let Some(next_hop) = route.next_hop.as_deref().filter(|value| !value.is_empty()) {
                entry.insert("next-hop".to_string(), owned_value(next_hop.to_string())?);
            }
            if let Some(metric) = route.metric {
                entry.insert("metric".to_string(), owned_value(metric)?);
            }
            if let Some(table) = route.table {
                entry.insert("table".to_string(), owned_value(table)?);
            }
            Ok(entry)
        })
        .collect()
}

fn apply_send_hostname(
    settings: &mut ConnectionSettings,
    section: &str,
    enabled: bool,
) -> Result<()> {
    settings
        .entry(section.to_string())
        .or_default()
        .insert("dhcp-send-hostname".to_string(), owned_value(enabled)?);
    Ok(())
}

fn normalized_key_mgmt(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "" | "--" | "open" => "open".to_string(),
        "none" | "wep" => "wep".to_string(),
        "psk" | "wpa-personal" | "wpa_psk" | "wpa-psk" => "wpa-psk".to_string(),
        "sae" => "sae".to_string(),
        "owe" => "owe".to_string(),
        "802.1x" | "802-1x" | "enterprise" | "wpa-eap" => "wpa-eap".to_string(),
        "suite-b" | "wpa-eap-suite-b-192" => "wpa-eap-suite-b-192".to_string(),
        other => other.to_string(),
    }
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

fn insert_required_string(
    settings: &mut HashMap<String, OwnedValue>,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        bail!("enterprise Wi-Fi field '{key}' is required")
    };
    settings.insert(key.to_string(), owned_value(value.to_string())?);
    Ok(())
}

fn insert_optional_string(
    settings: &mut HashMap<String, OwnedValue>,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        settings.insert(key.to_string(), owned_value(value.to_string())?);
    }
    Ok(())
}

fn insert_optional_strings(
    settings: &mut HashMap<String, OwnedValue>,
    values: &[(&str, Option<&str>)],
) -> Result<()> {
    values
        .iter()
        .try_for_each(|(key, value)| insert_optional_string(settings, key, *value))
}

fn insert_optional_u32(
    settings: &mut HashMap<String, OwnedValue>,
    key: &str,
    value: Option<u32>,
) -> Result<()> {
    if let Some(value) = value {
        settings.insert(key.to_string(), owned_value(value)?);
    }
    Ok(())
}

fn insert_optional_u32s(
    settings: &mut HashMap<String, OwnedValue>,
    values: &[(&str, Option<u32>)],
) -> Result<()> {
    values
        .iter()
        .try_for_each(|(key, value)| insert_optional_u32(settings, key, *value))
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
    use super::{
        cloned_wifi_connection_settings, enterprise_wifi_connection_settings,
        hidden_wifi_connection_settings, owe_wifi_connection_settings, psk_key_mgmt,
        psk_wifi_connection_settings, validate_wep_key, validate_wpa_psk,
    };
    use crate::model::{
        AccessPoint, EnterpriseAuth, NM_AP_SEC_KEY_MGMT_802_1X, NM_AP_SEC_KEY_MGMT_PSK,
        NM_AP_SEC_KEY_MGMT_SAE, TargetIpAddress, TargetIpRoute, TargetIpSettings,
        TargetProfileSettings, WepKeyType, example_connect_target,
    };

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
    fn owe_wifi_settings_include_key_mgmt_without_secret() {
        let settings = owe_wifi_connection_settings().expect("settings");

        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "key-mgmt"))
                .as_deref(),
            Some("owe")
        );
        assert!(
            settings
                .get("802-11-wireless-security")
                .is_some_and(|section| !section.contains_key("psk"))
        );
    }

    #[test]
    fn hidden_key_mgmt_hint_controls_security_shape() {
        let mut target = example_connect_target(true);
        target.key_mgmt = Some("sae".to_string());
        let settings =
            hidden_wifi_connection_settings(&target, Some("secret123"), None).expect("settings");

        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "key-mgmt"))
                .as_deref(),
            Some("sae")
        );

        target.key_mgmt = Some("open".to_string());
        let settings = hidden_wifi_connection_settings(&target, None, None).expect("settings");
        assert!(!settings.contains_key("802-11-wireless-security"));
    }

    #[test]
    fn cloned_profile_settings_replace_secret_and_preserve_profile_options() {
        let mut target = example_connect_target(true);
        target.profile = TargetProfileSettings {
            autoconnect: Some(false),
            autoconnect_priority: Some(20),
            metered: Some("no".to_string()),
            cloned_mac_address: Some("stable".to_string()),
            send_hostname: Some(false),
            ipv4: Some(TargetIpSettings {
                addresses: vec![TargetIpAddress {
                    address: "192.0.2.10".to_string(),
                    prefix: 24,
                }],
                gateway: Some("192.0.2.1".to_string()),
                dns: vec!["1.1.1.1".to_string(), "9.9.9.9".to_string()],
                routes: vec![TargetIpRoute {
                    dest: "198.51.100.0".to_string(),
                    prefix: 24,
                    next_hop: Some("192.0.2.1".to_string()),
                    metric: Some(20),
                    table: None,
                }],
                route_metric: Some(50),
                ignore_auto_dns: Some(true),
                dns_search: vec!["example.test".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let existing = super::base_wifi_connection_settings("Example", b"Example", false)
            .expect("base settings");
        let settings = cloned_wifi_connection_settings(
            existing,
            &target,
            &test_ap(NM_AP_SEC_KEY_MGMT_PSK),
            Some("secret123"),
            None,
        )
        .expect("settings");

        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "psk"))
                .as_deref(),
            Some("secret123")
        );
        assert_eq!(
            settings
                .get("connection")
                .and_then(|section| setting_bool(section, "autoconnect")),
            Some(false)
        );
        assert_eq!(
            settings
                .get("802-11-wireless")
                .and_then(|section| setting_string(section, "cloned-mac-address"))
                .as_deref(),
            Some("stable")
        );
        assert_eq!(
            settings
                .get("ipv4")
                .and_then(|section| setting_string(section, "method"))
                .as_deref(),
            Some("manual")
        );
        assert_eq!(
            settings
                .get("ipv4")
                .and_then(|section| setting_string(section, "gateway"))
                .as_deref(),
            Some("192.0.2.1")
        );
        assert_eq!(
            settings
                .get("ipv4")
                .and_then(|section| setting_i64(section, "route-metric")),
            Some(50)
        );
        assert_eq!(
            settings
                .get("ipv4")
                .and_then(|section| setting_string_vec(section, "dns-data")),
            Some(vec!["1.1.1.1".to_string(), "9.9.9.9".to_string()])
        );
        let address_data = settings
            .get("ipv4")
            .and_then(|section| setting_map_vec(section, "address-data"))
            .expect("address-data");
        assert_eq!(
            setting_string(&address_data[0], "address").as_deref(),
            Some("192.0.2.10")
        );
        assert_eq!(setting_u32(&address_data[0], "prefix"), Some(24));
        let route_data = settings
            .get("ipv4")
            .and_then(|section| setting_map_vec(section, "route-data"))
            .expect("route-data");
        assert_eq!(
            setting_string(&route_data[0], "dest").as_deref(),
            Some("198.51.100.0")
        );
        assert_eq!(
            setting_string(&route_data[0], "next-hop").as_deref(),
            Some("192.0.2.1")
        );
    }

    #[test]
    fn enterprise_wifi_settings_include_8021x_credentials() {
        let auth = EnterpriseAuth {
            eap: vec!["peap".to_string()],
            identity: Some("laufan".to_string()),
            anonymous_identity: None,
            password: None,
            phase2_auth: Some("mschapv2".to_string()),
            ..Default::default()
        };
        let settings = enterprise_wifi_connection_settings(
            &test_ap(NM_AP_SEC_KEY_MGMT_802_1X),
            &auth,
            Some("secret123"),
        )
        .expect("settings");

        assert_eq!(
            settings
                .get("802-11-wireless-security")
                .and_then(|section| setting_string(section, "key-mgmt"))
                .as_deref(),
            Some("wpa-eap")
        );
        assert_eq!(
            settings
                .get("802-1x")
                .and_then(|section| setting_string(section, "identity"))
                .as_deref(),
            Some("laufan")
        );
        assert_eq!(
            settings
                .get("802-1x")
                .and_then(|section| setting_string(section, "password"))
                .as_deref(),
            Some("secret123")
        );
        assert_eq!(
            settings
                .get("802-1x")
                .and_then(|section| setting_string(section, "phase2-auth"))
                .as_deref(),
            Some("mschapv2")
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
            channel: 1,
            band: "2.4 GHz".to_string(),
            mode: "Infra".to_string(),
            max_bitrate_mbps: 0,
            bandwidth_mhz: 0,
            ssid_hex: "4578616d706c65".to_string(),
            wpa_flags_label: "(none)".to_string(),
            rsn_flags_label: "(none)".to_string(),
            bssid: "00:11:22:33:44:55".to_string(),
            last_seen: 0,
            last_seen_age_ms: None,
            path: "/ap".to_string(),
            device_path: "/device".to_string(),
            device_iface: "wlan0".to_string(),
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

    fn setting_bool(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<bool> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }

    fn setting_i64(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<i64> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }

    fn setting_u32(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<u32> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }

    fn setting_string_vec(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<Vec<String>> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }

    fn setting_map_vec(
        settings: &std::collections::HashMap<String, zvariant::OwnedValue>,
        key: &str,
    ) -> Option<Vec<std::collections::HashMap<String, zvariant::OwnedValue>>> {
        settings.get(key)?.try_clone().ok()?.try_into().ok()
    }
}
