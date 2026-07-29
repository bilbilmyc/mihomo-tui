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
}

#[derive(Debug, Deserialize)]
struct MihomoConfig {
    #[serde(rename = "external-controller")]
    external_controller: Option<String>,
    secret: Option<String>,
}

pub fn discover(explicit_path: Option<&Path>) -> Option<ConnectionInfo> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit_path {
        candidates.push(path.to_path_buf());
    }
    candidates.extend([
        PathBuf::from("/etc/mihomo/config.yaml"),
        PathBuf::from("/etc/mihomo/config.yml"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home).join(".config/mihomo");
        candidates.push(home.join("config.yaml"));
        candidates.push(home.join("config.yml"));
    }
    candidates.into_iter().find_map(|path| read_config(&path))
}

fn read_config(path: &Path) -> Option<ConnectionInfo> {
    let content = fs::read_to_string(path).ok()?;
    let config: MihomoConfig = serde_yaml::from_str(&content).ok()?;
    let controller = normalize_controller(&config.external_controller?);
    Some(ConnectionInfo {
        controller,
        secret: config.secret.filter(|value| !value.is_empty()),
        config_path: path.to_path_buf(),
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
}
