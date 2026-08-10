use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub controller: String,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MihomoConfig {
    #[serde(rename = "external-controller")]
    external_controller: Option<String>,
    secret: Option<String>,
}

pub fn discover(path: &Path) -> Option<ConnectionInfo> {
    read_config(path)
}

fn read_config(path: &Path) -> Option<ConnectionInfo> {
    let content = fs::read_to_string(path).ok()?;
    let document: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let profile = crate::workspace::config_profile(&document).ok()?;
    let config: MihomoConfig = serde_yaml::from_value(profile.clone()).ok()?;
    let controller = normalize_controller(&config.external_controller?);
    Some(ConnectionInfo {
        controller,
        secret: config.secret.filter(|value| !value.is_empty()),
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
    fn reads_an_explicit_config() {
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

        let info = read_config(&path).unwrap();

        assert_eq!(info.controller, "http://127.0.0.1:9090");
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

        assert!(discover(&missing).is_none());
    }

    #[test]
    fn discovers_controller_credentials_from_a_workspace_profile() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mihomo-tui-workspace-discovery-{}-{unique}.yaml",
            std::process::id()
        ));
        fs::write(
            &path,
            "kind: mihomo-tui/v1\nbackend: mihomo\nprofile:\n  external-controller: 0.0.0.0:19093\n  secret: workspace-secret\n",
        )
        .unwrap();

        let info = read_config(&path).unwrap();

        assert_eq!(info.controller, "http://127.0.0.1:19093");
        assert_eq!(info.secret.as_deref(), Some("workspace-secret"));
        fs::remove_file(path).unwrap();
    }
}
