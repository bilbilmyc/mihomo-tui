#[cfg(test)]
mod tests {
    use serde_yaml::Value;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parses_runtime_settings_rules_and_providers() {
        let snapshot = super::parse_config(
            r#"
mixed-port: 7890
mode: rule
tun: { enable: true }
dns: { enable: true, enhanced-mode: fake-ip }
proxy-providers:
  airport:
    type: http
    url: https://example.com/sub
rules:
  - DOMAIN-SUFFIX,example.com,Proxy
"#,
        )
        .unwrap();

        assert_eq!(snapshot.mixed_port, Some(7890));
        assert!(snapshot.tun.enable);
        assert!(snapshot.dns.enable);
        assert_eq!(snapshot.providers[0].name, "airport");
        assert_eq!(snapshot.rules[0].kind, "DOMAIN-SUFFIX");
    }

    #[test]
    fn parses_editable_tun_and_dns_settings() {
        let snapshot = super::parse_config(
            r#"
tun:
  enable: true
  stack: mixed
  device: Mihomo
  auto-route: true
  auto-redirect: true
  strict-route: true
  auto-detect-interface: false
  dns-hijack: [any:53, tcp://any:53]
  mtu: 1500
  route-exclude-address: [192.168.0.0/16, fc00::/7]
dns:
  enable: true
  listen: 0.0.0.0:1053
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-range6: fdfe:dcba:9876::1/64
  fake-ip-filter-mode: whitelist
  ipv6: true
  prefer-h3: true
  respect-rules: true
"#,
        )
        .unwrap();

        assert_eq!(snapshot.tun.stack, "mixed");
        assert_eq!(snapshot.tun.device.as_deref(), Some("Mihomo"));
        assert!(snapshot.tun.auto_route);
        assert!(snapshot.tun.auto_redirect);
        assert!(snapshot.tun.strict_route);
        assert!(!snapshot.tun.auto_detect_interface);
        assert_eq!(snapshot.tun.dns_hijack, ["any:53", "tcp://any:53"]);
        assert_eq!(snapshot.tun.mtu, Some(1500));
        assert_eq!(
            snapshot.tun.route_exclude_address,
            ["192.168.0.0/16", "fc00::/7"]
        );
        assert_eq!(snapshot.dns.listen.as_deref(), Some("0.0.0.0:1053"));
        assert_eq!(snapshot.dns.enhanced_mode, "fake-ip");
        assert_eq!(snapshot.dns.fake_ip_range.as_deref(), Some("198.18.0.1/16"));
        assert_eq!(
            snapshot.dns.fake_ip_range6.as_deref(),
            Some("fdfe:dcba:9876::1/64")
        );
        assert_eq!(snapshot.dns.fake_ip_filter_mode, "whitelist");
        assert!(snapshot.dns.ipv6);
        assert!(snapshot.dns.prefer_h3);
        assert!(snapshot.dns.respect_rules);
    }

    #[test]
    fn applying_network_settings_preserves_unmanaged_fields() {
        let mut document: Value = serde_yaml::from_str(
            r#"
tun:
  gso: true
  device: old-device
dns:
  nameserver: [https://dns.alidns.com/dns-query]
  listen: 127.0.0.1:53
"#,
        )
        .unwrap();
        let tun = super::TunSettings {
            enable: true,
            stack: "system".into(),
            auto_route: true,
            auto_redirect: false,
            strict_route: false,
            auto_detect_interface: true,
            dns_hijack: vec!["any:53".into()],
            mtu: Some(9000),
            route_exclude_address: Vec::new(),
            device: None,
        };
        let dns = super::DnsSettings {
            enable: true,
            listen: None,
            enhanced_mode: "redir-host".into(),
            fake_ip_range: None,
            fake_ip_range6: None,
            fake_ip_filter_mode: "blacklist".into(),
            ipv6: false,
            prefer_h3: false,
            respect_rules: false,
        };

        super::apply_tun_settings(&mut document, &tun).unwrap();
        super::apply_dns_settings(&mut document, &dns).unwrap();

        let root = document.as_mapping().unwrap();
        let tun = root["tun"].as_mapping().unwrap();
        let dns = root["dns"].as_mapping().unwrap();
        assert_eq!(tun["gso"], Value::Bool(true));
        assert!(!tun.contains_key("device"));
        assert!(!tun.contains_key("route-exclude-address"));
        assert_eq!(tun["dns-hijack"][0], Value::String("any:53".into()));
        assert_eq!(dns["nameserver"][0], "https://dns.alidns.com/dns-query");
        assert!(!dns.contains_key("listen"));
        assert!(!dns.contains_key("fake-ip-range"));
    }

    #[test]
    fn saves_and_reloads_tun_and_dns_settings_through_mihomo_validation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-network-settings-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(
            &path,
            r#"
mixed-port: 7890
mode: rule
tun: { enable: false }
dns: { enable: false }
proxies: []
proxy-groups: []
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap();
        let tun = super::TunSettings {
            enable: false,
            stack: "mixed".into(),
            device: Some("Mihomo".into()),
            auto_route: true,
            auto_redirect: true,
            strict_route: true,
            auto_detect_interface: true,
            dns_hijack: vec!["any:53".into(), "tcp://any:53".into()],
            mtu: Some(1500),
            route_exclude_address: vec!["192.168.0.0/16".into()],
        };
        let dns = super::DnsSettings {
            enable: false,
            listen: Some("127.0.0.1:1053".into()),
            enhanced_mode: "fake-ip".into(),
            fake_ip_range: Some("198.18.0.1/16".into()),
            fake_ip_range6: Some("fdfe:dcba:9876::1/64".into()),
            fake_ip_filter_mode: "blacklist".into(),
            ipv6: true,
            prefer_h3: false,
            respect_rules: false,
        };

        let tun_backup = super::save_tun_settings(&path, &tun).unwrap();
        let dns_backup = super::save_dns_settings(&path, &dns).unwrap();
        let snapshot = super::load(&path).unwrap();

        assert_eq!(snapshot.tun, tun);
        assert_eq!(snapshot.dns, dns);
        fs::remove_file(tun_backup).unwrap();
        fs::remove_file(dns_backup).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn counts_rules_that_keep_tun_traffic_direct() {
        let snapshot = super::parse_config(
            r#"
rules:
  - GEOIP,CN,DIRECT
  - DOMAIN-SUFFIX,example.com,Proxy
  - MATCH,DIRECT
"#,
        )
        .unwrap();

        assert_eq!(snapshot.direct_rule_count(), 2);
    }

    #[test]
    fn serializes_match_rules_with_two_parts() {
        let rule = super::ConfigRule {
            kind: "MATCH".into(),
            value: "all".into(),
            action: "DIRECT".into(),
            extra: Vec::new(),
            raw: None,
        };

        assert_eq!(super::serialize_rule(&rule), "MATCH,DIRECT");
    }

    #[test]
    fn preserves_complete_rule_syntax_when_serializing() {
        let snapshot = super::parse_config(
            r#"
rules:
  - IP-CIDR,91.108.4.0/22,Proxy,no-resolve
  - AND,((DOMAIN,example.com),(NETWORK,TCP)),REJECT-DROP
"#,
        )
        .unwrap();

        assert_eq!(
            super::serialize_rule(&snapshot.rules[0]),
            "IP-CIDR,91.108.4.0/22,Proxy,no-resolve"
        );
        assert_eq!(
            super::serialize_rule(&snapshot.rules[1]),
            "AND,((DOMAIN,example.com),(NETWORK,TCP)),REJECT-DROP"
        );
    }

    #[test]
    fn serializes_edited_rules_with_their_additional_parameters() {
        let rule = super::ConfigRule {
            kind: "IP-CIDR".into(),
            value: "10.0.0.0/8".into(),
            action: "DIRECT".into(),
            extra: vec!["no-resolve".into()],
            raw: None,
        };

        assert_eq!(
            super::serialize_rule(&rule),
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve"
        );
    }

    #[cfg(unix)]
    #[test]
    fn validated_write_preserves_existing_config_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-permissions-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(&path, "mixed-port: 7890\nrules:\n  - MATCH,DIRECT\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let document: Value = serde_yaml::from_str(
            "mixed-port: 7890\nrules:\n  - DOMAIN,example.com,DIRECT\n  - MATCH,DIRECT\n",
        )
        .unwrap();

        let backup = super::write_validated(&path, &document).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(backup).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn validation_errors_include_mihomo_stdout_diagnostics() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-validation-error-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(&path, "rules:\n  - MATCH,DIRECT\n").unwrap();
        let invalid: Value = serde_yaml::from_str("rules: [MATCH,DIRECT]\n").unwrap();

        let error = super::write_validated(&path, &invalid).unwrap_err();

        assert!(
            error.contains("format invalid"),
            "unexpected error: {error}"
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn maps_proxy_providers_to_every_group_that_uses_them() {
        let snapshot = super::parse_config(
            r#"
proxy-providers:
  sbyun: { type: http, url: https://example.com/sub }
proxy-groups:
  - name: Main
    type: select
    use: [sbyun]
  - name: Auto
    type: url-test
    use: [sbyun]
"#,
        )
        .unwrap();

        assert_eq!(snapshot.provider_groups["sbyun"], ["Main", "Auto"]);
        assert_eq!(snapshot.proxy_groups, ["Main", "Auto"]);
    }

    #[test]
    fn rejects_subscription_urls_without_a_host_before_reading_the_config() {
        let error = super::add_http_provider(
            std::path::Path::new("/path/that/does/not/exist"),
            "airport",
            "http://",
        )
        .unwrap_err();

        assert!(error.contains("valid HTTP URL"));
    }

    #[test]
    fn updates_existing_http_provider_without_duplicating_its_group() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-provider-update-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(
            &path,
            r#"
mixed-port: 7890
mode: rule
proxy-providers:
  airport:
    type: http
    url: https://old.example.com/sub
    path: ./proxy-providers/airport.yaml
    interval: 3600
    health-check:
      enable: true
      url: https://www.gstatic.com/generate_204
      interval: 300
proxy-groups:
  - name: Main
    type: select
    use: [airport]
    proxies: [DIRECT]
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap();

        let backup =
            super::add_http_provider(&path, "airport", "https://new.example.com/sub").unwrap();
        let document: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let root = document.as_mapping().unwrap();
        let provider = root["proxy-providers"]["airport"].as_mapping().unwrap();

        assert_eq!(provider["url"], "https://new.example.com/sub");
        assert_eq!(provider["path"], "./proxy-providers/airport.yaml");
        assert_eq!(provider["interval"], 3600);
        assert_eq!(provider["health-check"]["interval"], 300);
        assert_eq!(root["proxy-groups"].as_sequence().unwrap().len(), 1);
        fs::remove_file(backup).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn adds_http_provider_with_a_distinct_proxy_group_name() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-provider-add-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(
            &path,
            r#"
mixed-port: 7890
mode: rule
proxy-providers: {}
proxy-groups:
  - name: airport-select
    type: select
    proxies: [DIRECT]
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap();

        let backup =
            super::add_http_provider(&path, "airport", "https://subscriptions.example.com/sub")
                .unwrap();
        let snapshot = super::load(&path).unwrap();

        assert_eq!(snapshot.providers[0].name, "airport");
        assert_eq!(snapshot.provider_groups["airport"], ["airport-select-2"]);
        fs::remove_file(backup).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn refuses_to_replace_a_file_provider_with_an_http_provider() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-file-provider-update-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(
            &path,
            r#"
proxy-providers:
  airport:
    type: file
    path: ./proxy-providers/airport.yaml
proxy-groups: []
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap();

        let error =
            super::add_http_provider(&path, "airport", "https://subscriptions.example.com/sub")
                .unwrap_err();

        assert_eq!(error, "Provider airport is not an HTTP provider");
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
use serde_yaml::{Mapping, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions, Permissions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunSettings {
    pub enable: bool,
    pub stack: String,
    pub device: Option<String>,
    pub auto_route: bool,
    pub auto_redirect: bool,
    pub strict_route: bool,
    pub auto_detect_interface: bool,
    pub dns_hijack: Vec<String>,
    pub mtu: Option<u32>,
    pub route_exclude_address: Vec<String>,
}

impl Default for TunSettings {
    fn default() -> Self {
        Self {
            enable: false,
            stack: "gvisor".into(),
            device: None,
            auto_route: false,
            auto_redirect: false,
            strict_route: false,
            auto_detect_interface: false,
            dns_hijack: Vec::new(),
            mtu: None,
            route_exclude_address: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsSettings {
    pub enable: bool,
    pub listen: Option<String>,
    pub enhanced_mode: String,
    pub fake_ip_range: Option<String>,
    pub fake_ip_range6: Option<String>,
    pub fake_ip_filter_mode: String,
    pub ipv6: bool,
    pub prefer_h3: bool,
    pub respect_rules: bool,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self {
            enable: false,
            listen: None,
            enhanced_mode: "redir-host".into(),
            fake_ip_range: None,
            fake_ip_range6: None,
            fake_ip_filter_mode: "blacklist".into(),
            ipv6: false,
            prefer_h3: false,
            respect_rules: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConfigSnapshot {
    pub mixed_port: Option<u16>,
    pub mode: String,
    pub tun: TunSettings,
    pub dns: DnsSettings,
    pub rules: Vec<ConfigRule>,
    pub providers: Vec<Provider>,
    pub proxy_groups: Vec<String>,
    pub provider_groups: BTreeMap<String, Vec<String>>,
}

impl ConfigSnapshot {
    pub fn direct_rule_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| matches!(rule.action.as_str(), "DIRECT" | "DIRECT-OUT"))
            .count()
    }
}

#[derive(Debug, Clone)]
pub struct ConfigRule {
    pub kind: String,
    pub value: String,
    pub action: String,
    pub extra: Vec<String>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub kind: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub interval: Option<u64>,
}

pub fn load(path: &Path) -> Result<ConfigSnapshot, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_config(&content)
}

pub fn save_rules(path: &Path, rules: &[ConfigRule]) -> Result<PathBuf, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: Value = serde_yaml::from_str(&content).map_err(|error| error.to_string())?;
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    let entries = rules
        .iter()
        .map(|rule| Value::String(serialize_rule(rule)))
        .collect();
    root.insert(Value::String("rules".into()), Value::Sequence(entries));
    write_validated(path, &document)
}

pub fn add_http_provider(path: &Path, name: &str, url: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > 48
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("Provider name must use letters, digits, - or _, up to 48 characters".into());
    }
    let parsed_url = reqwest::Url::parse(url)
        .map_err(|_| "Subscription must be a valid HTTP URL".to_string())?;
    if !matches!(parsed_url.scheme(), "https" | "http")
        || parsed_url.host_str().is_none()
        || url.len() > 4_096
    {
        return Err("Subscription must be a valid HTTP URL".into());
    }
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: Value = serde_yaml::from_str(&content).map_err(|error| error.to_string())?;
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    let providers = root
        .entry(Value::String("proxy-providers".into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| "proxy-providers must be a mapping".to_string())?;
    let provider_key = Value::String(name.into());
    let updating = providers.contains_key(&provider_key);
    if let Some(provider) = providers.get_mut(&provider_key) {
        let provider = provider
            .as_mapping_mut()
            .ok_or_else(|| format!("Provider {name} must be a mapping"))?;
        if field(provider, "type").and_then(Value::as_str) != Some("http") {
            return Err(format!("Provider {name} is not an HTTP provider"));
        }
        provider.insert(Value::String("url".into()), Value::String(url.into()));
    } else {
        let mut provider = Mapping::new();
        provider.insert(Value::String("type".into()), Value::String("http".into()));
        provider.insert(Value::String("url".into()), Value::String(url.into()));
        provider.insert(
            Value::String("path".into()),
            Value::String(format!("./proxy-providers/{name}.yaml")),
        );
        provider.insert(
            Value::String("interval".into()),
            Value::Number(86_400.into()),
        );
        providers.insert(provider_key, Value::Mapping(provider));
    }
    let provider_names: Vec<String> = providers
        .keys()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if updating {
        return write_validated(path, &document);
    }
    let groups = root
        .entry(Value::String("proxy-groups".into()))
        .or_insert_with(|| Value::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(|| "proxy-groups must be a list".to_string())?;
    let group_name_in_use = |candidate: &str| {
        provider_names.iter().any(|name| name == candidate)
            || groups.iter().any(|group| {
                group
                    .as_mapping()
                    .and_then(|group| field(group, "name"))
                    .and_then(Value::as_str)
                    == Some(candidate)
            })
    };
    let group_name = (1_u64..)
        .map(|suffix| {
            if suffix == 1 {
                format!("{name}-select")
            } else {
                format!("{name}-select-{suffix}")
            }
        })
        .find(|candidate| !group_name_in_use(candidate))
        .ok_or_else(|| "Could not allocate a proxy group name".to_string())?;
    let mut group = Mapping::new();
    group.insert(Value::String("name".into()), Value::String(group_name));
    group.insert(Value::String("type".into()), Value::String("select".into()));
    group.insert(
        Value::String("use".into()),
        Value::Sequence(vec![Value::String(name.into())]),
    );
    group.insert(
        Value::String("proxies".into()),
        Value::Sequence(vec![Value::String("DIRECT".into())]),
    );
    groups.push(Value::Mapping(group));
    write_validated(path, &document)
}

pub fn save_tun_settings(path: &Path, settings: &TunSettings) -> Result<PathBuf, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: Value = serde_yaml::from_str(&content).map_err(|error| error.to_string())?;
    apply_tun_settings(&mut document, settings)?;
    write_validated(path, &document)
}

pub fn save_dns_settings(path: &Path, settings: &DnsSettings) -> Result<PathBuf, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: Value = serde_yaml::from_str(&content).map_err(|error| error.to_string())?;
    apply_dns_settings(&mut document, settings)?;
    write_validated(path, &document)
}

fn apply_tun_settings(document: &mut Value, settings: &TunSettings) -> Result<(), String> {
    if !matches!(settings.stack.as_str(), "system" | "gvisor" | "mixed") {
        return Err("TUN stack must be system, gvisor, or mixed".into());
    }
    let tun = editable_section(document, "tun")?;
    set_bool(tun, "enable", settings.enable);
    set_string(tun, "stack", &settings.stack);
    set_optional_string(tun, "device", settings.device.as_deref());
    set_bool(tun, "auto-route", settings.auto_route);
    set_bool(tun, "auto-redirect", settings.auto_redirect);
    set_bool(tun, "strict-route", settings.strict_route);
    set_bool(tun, "auto-detect-interface", settings.auto_detect_interface);
    set_string_list(tun, "dns-hijack", &settings.dns_hijack);
    set_optional_u32(tun, "mtu", settings.mtu);
    set_string_list(
        tun,
        "route-exclude-address",
        &settings.route_exclude_address,
    );
    Ok(())
}

fn apply_dns_settings(document: &mut Value, settings: &DnsSettings) -> Result<(), String> {
    if !matches!(settings.enhanced_mode.as_str(), "fake-ip" | "redir-host") {
        return Err("DNS enhanced mode must be fake-ip or redir-host".into());
    }
    if !matches!(
        settings.fake_ip_filter_mode.as_str(),
        "blacklist" | "whitelist" | "rule"
    ) {
        return Err("DNS fake IP filter mode must be blacklist, whitelist, or rule".into());
    }
    let dns = editable_section(document, "dns")?;
    set_bool(dns, "enable", settings.enable);
    set_optional_string(dns, "listen", settings.listen.as_deref());
    set_string(dns, "enhanced-mode", &settings.enhanced_mode);
    set_optional_string(dns, "fake-ip-range", settings.fake_ip_range.as_deref());
    set_optional_string(dns, "fake-ip-range6", settings.fake_ip_range6.as_deref());
    set_string(dns, "fake-ip-filter-mode", &settings.fake_ip_filter_mode);
    set_bool(dns, "ipv6", settings.ipv6);
    set_bool(dns, "prefer-h3", settings.prefer_h3);
    set_bool(dns, "respect-rules", settings.respect_rules);
    Ok(())
}

fn editable_section<'a>(document: &'a mut Value, name: &str) -> Result<&'a mut Mapping, String> {
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    root.entry(Value::String(name.into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| format!("{name} must be a mapping"))
}

fn set_bool(section: &mut Mapping, key: &str, value: bool) {
    section.insert(Value::String(key.into()), Value::Bool(value));
}

fn set_string(section: &mut Mapping, key: &str, value: &str) {
    section.insert(Value::String(key.into()), Value::String(value.into()));
}

fn set_optional_string(section: &mut Mapping, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        set_string(section, key, value);
    } else {
        section.remove(Value::String(key.into()));
    }
}

fn set_optional_u32(section: &mut Mapping, key: &str, value: Option<u32>) {
    if let Some(value) = value {
        section.insert(
            Value::String(key.into()),
            Value::Number(u64::from(value).into()),
        );
    } else {
        section.remove(Value::String(key.into()));
    }
}

fn set_string_list(section: &mut Mapping, key: &str, values: &[String]) {
    if values.is_empty() {
        section.remove(Value::String(key.into()));
    } else {
        section.insert(
            Value::String(key.into()),
            Value::Sequence(values.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn write_validated(path: &Path, document: &Value) -> Result<PathBuf, String> {
    let serialized = serde_yaml::to_string(document).map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    let original_permissions = fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    let stamp = unique_stamp()?;
    let candidate = parent.join(format!(".mihomo-tui-{}-{stamp}.yaml", std::process::id()));
    write_new_file(&candidate, serialized.as_bytes(), original_permissions)?;
    let validation = match crate::runtime::validate_config(&candidate, parent) {
        Ok(validation) => validation,
        Err(error) => {
            let _ = fs::remove_file(&candidate);
            return Err(error);
        }
    };
    if !validation.status.success() {
        let _ = fs::remove_file(&candidate);
        return Err(format!(
            "Mihomo rejected candidate config: {}",
            validation_diagnostic(&validation)
        ));
    }
    let backup = parent.join(format!("{}.{stamp}.mihomo-tui.bak", file_name(path)?));
    if let Err(error) = fs::copy(path, &backup) {
        let _ = fs::remove_file(&candidate);
        return Err(error.to_string());
    }
    if let Err(error) = restrict_backup_permissions(&backup) {
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_file(&backup);
        return Err(error);
    }
    if let Err(error) = fs::rename(&candidate, path) {
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_file(&backup);
        return Err(error.to_string());
    }
    sync_directory(parent)?;
    Ok(backup)
}

fn validation_diagnostic(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim(), stderr.trim()) {
        ("", "") => "no diagnostic output".into(),
        (stdout, "") => stdout.into(),
        ("", stderr) => stderr.into(),
        (stdout, stderr) => format!("{stderr}\n{stdout}"),
    }
}

pub fn restore_backup(path: &Path, backup: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    let content = fs::read(backup).map_err(|error| error.to_string())?;
    let permissions = fs::metadata(path)
        .or_else(|_| fs::metadata(backup))
        .map_err(|error| error.to_string())?
        .permissions();
    let candidate = parent.join(format!(
        ".mihomo-tui-restore-{}-{}.yaml",
        std::process::id(),
        unique_stamp()?
    ));
    write_new_file(&candidate, &content, permissions)?;
    if let Err(error) = fs::rename(&candidate, path) {
        let _ = fs::remove_file(&candidate);
        return Err(error.to_string());
    }
    sync_directory(parent)
}

fn unique_stamp() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())
        .map(|duration| duration.as_nanos())
}

fn write_new_file(path: &Path, content: &[u8], permissions: Permissions) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    if let Err(error) = file.write_all(content) {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    if let Err(error) = file.set_permissions(permissions) {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_backup_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn restrict_backup_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| "config path has no valid filename".to_string())
}

pub fn parse_config(content: &str) -> Result<ConfigSnapshot, String> {
    let document: Value = serde_yaml::from_str(content).map_err(|error| error.to_string())?;
    let root = document
        .as_mapping()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    let rules = field(root, "rules")
        .and_then(Value::as_sequence)
        .map(|items| items.iter().filter_map(parse_rule).collect())
        .unwrap_or_default();
    let providers = field(root, "proxy-providers")
        .and_then(Value::as_mapping)
        .map(parse_providers)
        .unwrap_or_default();
    let provider_groups = parse_provider_groups(root);
    let proxy_groups = parse_proxy_group_names(root);
    let tun = TunSettings {
        enable: nested_bool(root, "tun", "enable"),
        stack: nested_string(root, "tun", "stack").unwrap_or_else(|| "gvisor".into()),
        device: nested_string(root, "tun", "device"),
        auto_route: nested_bool(root, "tun", "auto-route"),
        auto_redirect: nested_bool(root, "tun", "auto-redirect"),
        strict_route: nested_bool(root, "tun", "strict-route"),
        auto_detect_interface: nested_bool(root, "tun", "auto-detect-interface"),
        dns_hijack: nested_string_list(root, "tun", "dns-hijack"),
        mtu: nested_u64(root, "tun", "mtu").and_then(|value| u32::try_from(value).ok()),
        route_exclude_address: nested_string_list(root, "tun", "route-exclude-address"),
    };
    let dns = DnsSettings {
        enable: nested_bool(root, "dns", "enable"),
        listen: nested_string(root, "dns", "listen"),
        enhanced_mode: nested_string(root, "dns", "enhanced-mode")
            .unwrap_or_else(|| "redir-host".into()),
        fake_ip_range: nested_string(root, "dns", "fake-ip-range"),
        fake_ip_range6: nested_string(root, "dns", "fake-ip-range6"),
        fake_ip_filter_mode: nested_string(root, "dns", "fake-ip-filter-mode")
            .unwrap_or_else(|| "blacklist".into()),
        ipv6: nested_bool(root, "dns", "ipv6"),
        prefer_h3: nested_bool(root, "dns", "prefer-h3"),
        respect_rules: nested_bool(root, "dns", "respect-rules"),
    };

    Ok(ConfigSnapshot {
        mixed_port: field(root, "mixed-port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok()),
        mode: field(root, "mode")
            .and_then(Value::as_str)
            .unwrap_or("rule")
            .to_string(),
        tun,
        dns,
        rules,
        providers,
        proxy_groups,
        provider_groups,
    })
}

fn field<'a>(map: &'a Mapping, name: &str) -> Option<&'a Value> {
    map.get(Value::String(name.into()))
}

fn nested_bool(root: &Mapping, section: &str, name: &str) -> bool {
    field(root, section)
        .and_then(Value::as_mapping)
        .and_then(|value| field(value, name))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn nested_string(root: &Mapping, section: &str, name: &str) -> Option<String> {
    field(root, section)
        .and_then(Value::as_mapping)
        .and_then(|value| field(value, name))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn nested_u64(root: &Mapping, section: &str, name: &str) -> Option<u64> {
    field(root, section)
        .and_then(Value::as_mapping)
        .and_then(|value| field(value, name))
        .and_then(Value::as_u64)
}

fn nested_string_list(root: &Mapping, section: &str, name: &str) -> Vec<String> {
    field(root, section)
        .and_then(Value::as_mapping)
        .and_then(|value| field(value, name))
        .and_then(Value::as_sequence)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_rule(value: &Value) -> Option<ConfigRule> {
    let raw = value.as_str()?;
    let parts = split_rule_fields(raw);
    if parts.len() == 2 && parts[0] == "MATCH" {
        return Some(ConfigRule {
            kind: parts[0].to_string(),
            value: "all".into(),
            action: parts[1].to_string(),
            extra: parts[2..].iter().map(|part| (*part).to_string()).collect(),
            raw: Some(raw.to_string()),
        });
    }
    if parts.len() < 3 {
        return None;
    }
    Some(ConfigRule {
        kind: parts[0].to_string(),
        value: parts[1].to_string(),
        action: parts[2].to_string(),
        extra: parts[3..].iter().map(|part| (*part).to_string()).collect(),
        raw: Some(raw.to_string()),
    })
}

fn serialize_rule(rule: &ConfigRule) -> String {
    if let Some(raw) = &rule.raw {
        return raw.clone();
    }
    let mut parts = if rule.kind == "MATCH" {
        vec![rule.kind.clone(), rule.action.clone()]
    } else {
        vec![rule.kind.clone(), rule.value.clone(), rule.action.clone()]
    };
    parts.extend(rule.extra.iter().cloned());
    parts.join(",")
}

fn split_rule_fields(raw: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in raw.char_indices() {
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                fields.push(raw[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    fields.push(raw[start..].trim());
    fields
}

fn parse_providers(providers: &Mapping) -> Vec<Provider> {
    let mut result: Vec<_> = providers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str()?.to_string();
            let entry = value.as_mapping()?;
            Some(Provider {
                name,
                kind: field(entry, "type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                url: field(entry, "url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                path: field(entry, "path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                interval: field(entry, "interval").and_then(Value::as_u64),
            })
        })
        .collect();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    result
}

fn parse_provider_groups(root: &Mapping) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    let Some(groups) = field(root, "proxy-groups").and_then(Value::as_sequence) else {
        return result;
    };
    for group in groups {
        let Some(group) = group.as_mapping() else {
            continue;
        };
        let Some(name) = field(group, "name").and_then(Value::as_str) else {
            continue;
        };
        let Some(providers) = field(group, "use").and_then(Value::as_sequence) else {
            continue;
        };
        for provider in providers.iter().filter_map(Value::as_str) {
            result
                .entry(provider.to_string())
                .or_default()
                .push(name.to_string());
        }
    }
    result
}

fn parse_proxy_group_names(root: &Mapping) -> Vec<String> {
    field(root, "proxy-groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_mapping)
        .filter_map(|group| field(group, "name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}
