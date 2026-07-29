#[cfg(test)]
mod tests {
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
        assert!(snapshot.tun_enabled);
        assert!(snapshot.dns_enabled);
        assert_eq!(snapshot.providers[0].name, "airport");
        assert_eq!(snapshot.rules[0].kind, "DOMAIN-SUFFIX");
    }
}
use serde_yaml::{Mapping, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default)]
pub struct ConfigSnapshot {
    pub mixed_port: Option<u16>,
    pub mode: String,
    pub tun_enabled: bool,
    pub dns_enabled: bool,
    pub dns_mode: String,
    pub rules: Vec<ConfigRule>,
    pub providers: Vec<Provider>,
}

#[derive(Debug, Clone)]
pub struct ConfigRule {
    pub kind: String,
    pub value: String,
    pub action: String,
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
        .map(|rule| Value::String(format!("{},{},{}", rule.kind, rule.value, rule.action)))
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
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Subscription URL must start with https:// or http://".into());
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
    if providers.contains_key(Value::String(name.into())) {
        return Err(format!("Provider {name} already exists"));
    }
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
    providers.insert(Value::String(name.into()), Value::Mapping(provider));
    let groups = root
        .entry(Value::String("proxy-groups".into()))
        .or_insert_with(|| Value::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(|| "proxy-groups must be a list".to_string())?;
    let mut group = Mapping::new();
    group.insert(Value::String("name".into()), Value::String(name.into()));
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

fn write_validated(path: &Path, document: &Value) -> Result<PathBuf, String> {
    let serialized = serde_yaml::to_string(document).map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let candidate = parent.join(format!(".mihomo-tui-{stamp}.yaml"));
    fs::write(&candidate, serialized).map_err(|error| error.to_string())?;
    let validation = Command::new("mihomo")
        .args(["-t", "-f"])
        .arg(&candidate)
        .args(["-d"])
        .arg(parent)
        .output()
        .map_err(|error| format!("could not run mihomo validation: {error}"))?;
    if !validation.status.success() {
        let _ = fs::remove_file(&candidate);
        return Err(format!(
            "Mihomo rejected candidate config: {}",
            String::from_utf8_lossy(&validation.stderr).trim()
        ));
    }
    let backup = parent.join(format!("{}.{stamp}.mihomo-tui.bak", file_name(path)?));
    fs::copy(path, &backup).map_err(|error| error.to_string())?;
    fs::rename(&candidate, path).map_err(|error| error.to_string())?;
    Ok(backup)
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
    let tun_enabled = nested_bool(root, "tun", "enable");
    let dns_enabled = nested_bool(root, "dns", "enable");
    let dns_mode = nested_string(root, "dns", "enhanced-mode").unwrap_or_else(|| "normal".into());

    Ok(ConfigSnapshot {
        mixed_port: field(root, "mixed-port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok()),
        mode: field(root, "mode")
            .and_then(Value::as_str)
            .unwrap_or("rule")
            .to_string(),
        tun_enabled,
        dns_enabled,
        dns_mode,
        rules,
        providers,
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

fn parse_rule(value: &Value) -> Option<ConfigRule> {
    let raw = value.as_str()?;
    let parts: Vec<_> = raw.split(',').map(str::trim).collect();
    if parts.len() < 3 {
        return None;
    }
    Some(ConfigRule {
        kind: parts[0].to_string(),
        value: parts[1].to_string(),
        action: parts[2].to_string(),
    })
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
