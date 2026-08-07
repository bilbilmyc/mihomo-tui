use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub controller: String,
    pub secret: Option<String>,
    pub config_path: PathBuf,
    pub origin: ConfigOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOrigin {
    Explicit,
    System,
    User,
}

#[derive(Debug, Deserialize)]
struct MihomoConfig {
    #[serde(rename = "external-controller")]
    external_controller: Option<String>,
    secret: Option<String>,
}

pub fn discover(explicit_path: Option<&Path>) -> Option<ConnectionInfo> {
    if let Some(path) = explicit_path {
        return read_config(path, ConfigOrigin::Explicit);
    }
    let mut candidates = Vec::new();
    candidates.extend([
        (
            PathBuf::from("/etc/mihomo/config.yaml"),
            ConfigOrigin::System,
        ),
        (
            PathBuf::from("/etc/mihomo/config.yml"),
            ConfigOrigin::System,
        ),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home).join(".config/mihomo");
        candidates.push((home.join("config.yaml"), ConfigOrigin::User));
        candidates.push((home.join("config.yml"), ConfigOrigin::User));
    }
    candidates
        .into_iter()
        .find_map(|(path, origin)| read_config(&path, origin))
}

pub fn user_config_exists() -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let home = PathBuf::from(home).join(".config/mihomo");
    [home.join("config.yaml"), home.join("config.yml")]
        .into_iter()
        .any(|path| fs::symlink_metadata(path).is_ok())
}

fn read_config(path: &Path, origin: ConfigOrigin) -> Option<ConnectionInfo> {
    let content = fs::read_to_string(path).ok()?;
    let config: MihomoConfig = serde_yaml::from_str(&content).ok()?;
    let controller = normalize_controller(&config.external_controller?);
    Some(ConnectionInfo {
        controller,
        secret: config.secret.filter(|value| !value.is_empty()),
        config_path: path.to_path_buf(),
        origin,
    })
}

fn normalize_controller(value: &str) -> String {
    let value = value.trim().trim_matches('\'');
    if let Some(port) = value.strip_prefix("0.0.0.0:") {
        return format!("http://127.0.0.1:{port}");
    }
    if value.starts_with("http://") || value.starts_with("https://") || value.starts_with("unix:") {
        value.to_string()
    } else {
        format!("http://{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn normalizes_bind_all_controller() {
        assert_eq!(
            normalize_controller("0.0.0.0:9090"),
            "http://127.0.0.1:9090"
        );
        assert_eq!(
            normalize_controller("127.0.0.1:9090"),
            "http://127.0.0.1:9090"
        );
    }

    #[test]
    fn discovered_config_keeps_its_trust_origin() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mihomo-tui-discovery-{}-{unique}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            "external-controller: 127.0.0.1:9090\nsecret: test-secret\n",
        )
        .unwrap();

        let info = read_config(&path, ConfigOrigin::Explicit).unwrap();

        assert_eq!(info.origin, ConfigOrigin::Explicit);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_explicit_config_does_not_fall_back() {
        let missing = std::env::temp_dir().join(format!(
            "mihomo-tui-missing-config-{}-{}.yaml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        assert!(discover(Some(&missing)).is_none());
    }
}
