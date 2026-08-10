use serde_yaml::{Mapping, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const WORKSPACE_KIND: &str = "mihomo-tui/v1";
pub const WORKSPACE_BACKEND: &str = "mihomo";
pub const DEFAULT_SOURCE_PATH: &str = "/etc/mihomo-tui/config.yaml";
pub const DEFAULT_RUNTIME_PATH: &str = "/etc/mihomo/config.yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initialization {
    Existing,
    Imported(PathBuf),
    Created,
}

pub fn initialize(source: &Path, import: Option<&Path>) -> Result<Initialization, String> {
    if fs::symlink_metadata(source).is_ok() {
        read_workspace_document(source)?;
        restrict_source_permissions(source)?;
        return Ok(Initialization::Existing);
    }

    let (profile, result) = match import.filter(|path| fs::symlink_metadata(path).is_ok()) {
        Some(path) => (
            read_raw_profile(path)?,
            Initialization::Imported(path.to_path_buf()),
        ),
        None => (default_profile()?, Initialization::Created),
    };
    let document = wrap_profile(profile);
    let content =
        serde_yaml::to_string(&document).map_err(|error| format!("无法序列化独立配置：{error}"))?;
    create_source(source, content.as_bytes())?;
    Ok(result)
}

pub fn read_workspace_document(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取独立配置 {}：{error}", path.display()))?;
    let document: Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("独立配置 {} 不是有效 YAML：{error}", path.display()))?;
    workspace_profile(&document)?;
    Ok(document)
}

pub fn load_profile(path: &Path) -> Result<Value, String> {
    let document = read_workspace_document(path)?;
    Ok(workspace_profile(&document)?.clone())
}

pub fn source_matches_runtime(source: &Path, target: &Path) -> Result<bool, String> {
    let profile = load_profile(source)?;
    let Ok(content) = fs::read_to_string(target) else {
        return Ok(false);
    };
    let Ok(runtime): Result<Value, _> = serde_yaml::from_str(&content) else {
        return Ok(false);
    };
    Ok(runtime.as_mapping().is_some() && runtime == profile)
}

pub fn config_profile(document: &Value) -> Result<&Value, String> {
    let root = document
        .as_mapping()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    if root.contains_key(Value::String("kind".into())) {
        workspace_profile(document)
    } else {
        Ok(document)
    }
}

pub fn config_profile_mut(document: &mut Value) -> Result<&mut Value, String> {
    let wrapped = document
        .as_mapping()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?
        .contains_key(Value::String("kind".into()));
    if !wrapped {
        return Ok(document);
    }
    validate_workspace_metadata(document)?;
    document
        .as_mapping_mut()
        .and_then(|root| root.get_mut(Value::String("profile".into())))
        .ok_or_else(|| "独立配置缺少 profile".to_string())
}

fn read_raw_profile(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取待导入配置 {}：{error}", path.display()))?;
    let profile: Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("待导入配置 {} 不是有效 YAML：{error}", path.display()))?;
    if profile.as_mapping().is_none() {
        return Err(format!("待导入配置 {} 的根节点必须是映射", path.display()));
    }
    Ok(profile)
}

fn wrap_profile(profile: Value) -> Value {
    let mut document = Mapping::new();
    document.insert(
        Value::String("kind".into()),
        Value::String(WORKSPACE_KIND.into()),
    );
    document.insert(
        Value::String("backend".into()),
        Value::String(WORKSPACE_BACKEND.into()),
    );
    document.insert(Value::String("profile".into()), profile);
    Value::Mapping(document)
}

fn workspace_profile(document: &Value) -> Result<&Value, String> {
    validate_workspace_metadata(document)?;
    let profile = document
        .as_mapping()
        .and_then(|root| root.get(Value::String("profile".into())))
        .ok_or_else(|| "独立配置缺少 profile".to_string())?;
    if profile.as_mapping().is_none() {
        return Err("独立配置的 profile 必须是映射".into());
    }
    Ok(profile)
}

fn validate_workspace_metadata(document: &Value) -> Result<(), String> {
    let root = document
        .as_mapping()
        .ok_or_else(|| "独立配置根节点必须是映射".to_string())?;
    let kind = root
        .get(Value::String("kind".into()))
        .and_then(Value::as_str)
        .ok_or_else(|| "独立配置缺少 kind".to_string())?;
    if kind != WORKSPACE_KIND {
        return Err(format!(
            "不支持的独立配置版本 {kind}，当前仅支持 {WORKSPACE_KIND}"
        ));
    }
    let backend = root
        .get(Value::String("backend".into()))
        .and_then(Value::as_str)
        .ok_or_else(|| "独立配置缺少 backend".to_string())?;
    if backend != WORKSPACE_BACKEND {
        return Err(format!("不支持的配置后端 {backend}"));
    }
    Ok(())
}

fn default_profile() -> Result<Value, String> {
    serde_yaml::from_str(
        r#"mixed-port: 7890
mode: rule
external-controller: 127.0.0.1:9093
proxy-providers: {}
proxy-groups: []
rules:
  - MATCH,DIRECT
"#,
    )
    .map_err(|error| format!("无法创建默认配置：{error}"))
}

fn create_source(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("独立配置路径 {} 没有父目录", path.display()))?;
    let parent_existed = fs::symlink_metadata(parent).is_ok();
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建独立配置目录 {}：{error}", parent.display()))?;
    #[cfg(unix)]
    if !parent_existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法设置独立配置目录权限 {}：{error}", parent.display()))?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| format!("无法创建独立配置 {}：{error}", path.display()))?;
    if let Err(error) = file.write_all(content).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(format!("无法写入独立配置 {}：{error}", path.display()));
    }
    restrict_source_permissions(path)
}

#[cfg(unix)]
fn restrict_source_permissions(path: &Path) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("无法设置独立配置权限 {}：{error}", path.display()))
}

#[cfg(not(unix))]
fn restrict_source_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mihomo-tui-workspace-{name}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_the_complete_legacy_mapping_without_changing_it() {
        let directory = TestDirectory::new("import");
        let legacy = directory.join("legacy.yaml");
        let source = directory.join("workspace/config.yaml");
        let legacy_content = r#"external-controller: 127.0.0.1:9093
secret: private
experimental:
  custom-feature:
    enabled: true
proxy-providers:
  airport:
    type: http
    url: https://example.com/private-subscription
rules: [MATCH,DIRECT]
"#;
        fs::write(&legacy, legacy_content).unwrap();

        let result = initialize(&source, Some(&legacy)).unwrap();

        assert_eq!(result, Initialization::Imported(legacy.clone()));
        assert_eq!(fs::read_to_string(&legacy).unwrap(), legacy_content);
        let document: Value = serde_yaml::from_str(&fs::read_to_string(&source).unwrap()).unwrap();
        assert_eq!(document["kind"], WORKSPACE_KIND);
        assert_eq!(document["backend"], WORKSPACE_BACKEND);
        assert_eq!(document["profile"]["secret"], "private");
        assert_eq!(
            document["profile"]["experimental"]["custom-feature"]["enabled"],
            true
        );
    }

    #[test]
    fn never_overwrites_an_existing_workspace() {
        let directory = TestDirectory::new("existing");
        let source = directory.join("config.yaml");
        let legacy = directory.join("legacy.yaml");
        let existing = r#"kind: mihomo-tui/v1
backend: mihomo
profile:
  mixed-port: 17890
  rules: [MATCH,DIRECT]
"#;
        fs::write(&source, existing).unwrap();
        fs::write(&legacy, "mixed-port: 27890\nrules: [MATCH,DIRECT]\n").unwrap();

        let result = initialize(&source, Some(&legacy)).unwrap();

        assert_eq!(result, Initialization::Existing);
        assert_eq!(fs::read_to_string(&source).unwrap(), existing);
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_workspace_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TestDirectory::new("permissions");
        let source = directory.join("nested/config.yaml");

        initialize(&source, None).unwrap();

        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(source.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn compares_the_source_profile_with_the_runtime_target() {
        let directory = TestDirectory::new("compare");
        let source = directory.join("config.yaml");
        let target = directory.join("runtime.yaml");
        fs::write(
            &source,
            "kind: mihomo-tui/v1\nbackend: mihomo\nprofile:\n  mode: rule\n  rules: [MATCH,DIRECT]\n",
        )
        .unwrap();

        assert!(!source_matches_runtime(&source, &target).unwrap());
        fs::write(&target, "rules: [MATCH,DIRECT]\nmode: rule\n").unwrap();
        assert!(source_matches_runtime(&source, &target).unwrap());
        fs::write(&target, "mode: global\nrules: [MATCH,DIRECT]\n").unwrap();
        assert!(!source_matches_runtime(&source, &target).unwrap());
        fs::write(&target, "not: [valid\n").unwrap();
        assert!(!source_matches_runtime(&source, &target).unwrap());
    }

    #[test]
    fn rejects_an_unsupported_existing_workspace() {
        let directory = TestDirectory::new("unsupported");
        let source = directory.join("config.yaml");
        fs::write(
            &source,
            "kind: mihomo-tui/v2\nbackend: mihomo\nprofile: {}\n",
        )
        .unwrap();

        let error = initialize(&source, None).unwrap_err();

        assert!(error.contains("mihomo-tui/v2"), "unexpected error: {error}");
    }
}
