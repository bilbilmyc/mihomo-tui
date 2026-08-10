use serde_yaml::{Mapping, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const WORKSPACE_KIND: &str = "mihomo-tui/v1";
pub const WORKSPACE_BACKEND: &str = "mihomo";
pub const DEFAULT_SOURCE_PATH: &str = "/etc/mihomo-tui/config.yaml";
pub const DEFAULT_IMPORT_PATH: &str = "/etc/mihomo/config.yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initialization {
    Existing,
    Imported(PathBuf),
    Created,
    MigratedWrapper(PathBuf),
}

pub fn initialize(source: &Path, import: Option<&Path>) -> Result<Initialization, String> {
    if let Ok(metadata) = fs::symlink_metadata(source) {
        if !metadata.file_type().is_file() {
            return Err(format!(
                "独立配置 {} 必须是普通文件，不能是符号链接",
                source.display()
            ));
        }
        let document = read_document(source, "独立配置")?;
        if has_workspace_kind(
            document
                .as_mapping()
                .ok_or_else(|| "独立配置根节点必须是映射".to_string())?,
        ) {
            let profile = workspace_profile(&document)?.clone();
            let backup = replace_with_native_profile(source, &profile)?;
            return Ok(Initialization::MigratedWrapper(backup));
        }
        require_mapping(&document, "独立配置")?;
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
    let content =
        serde_yaml::to_string(&profile).map_err(|error| format!("无法序列化独立配置：{error}"))?;
    create_source(source, content.as_bytes())?;
    Ok(result)
}

fn read_document(path: &Path, label: &str) -> Result<Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取{label} {}：{error}", path.display()))?;
    let document: Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("{label} {} 不是有效 YAML：{error}", path.display()))?;
    Ok(document)
}

pub fn load_profile(path: &Path) -> Result<Value, String> {
    let document = read_document(path, "配置")?;
    Ok(config_profile(&document)?.clone())
}

pub fn config_profile(document: &Value) -> Result<&Value, String> {
    let root = document
        .as_mapping()
        .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?;
    if has_workspace_kind(root) {
        workspace_profile(document)
    } else {
        Ok(document)
    }
}

pub fn config_profile_mut(document: &mut Value) -> Result<&mut Value, String> {
    let wrapped = has_workspace_kind(
        document
            .as_mapping()
            .ok_or_else(|| "Mihomo config root must be a mapping".to_string())?,
    );
    if !wrapped {
        return Ok(document);
    }
    validate_workspace_metadata(document)?;
    document
        .as_mapping_mut()
        .and_then(|root| root.get_mut(Value::String("profile".into())))
        .ok_or_else(|| "独立配置缺少 profile".to_string())
}

fn has_workspace_kind(root: &Mapping) -> bool {
    root.get(Value::String("kind".into()))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.starts_with("mihomo-tui/"))
}

fn read_raw_profile(path: &Path) -> Result<Value, String> {
    let profile = read_document(path, "待导入配置")?;
    require_mapping(&profile, "待导入配置")?;
    Ok(profile)
}

fn require_mapping(document: &Value, label: &str) -> Result<(), String> {
    if document.as_mapping().is_some() {
        Ok(())
    } else {
        Err(format!("{label}的根节点必须是映射"))
    }
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

fn replace_with_native_profile(path: &Path, profile: &Value) -> Result<PathBuf, String> {
    let content = serde_yaml::to_string(profile)
        .map_err(|error| format!("无法序列化迁移后的配置：{error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("独立配置路径 {} 没有父目录", path.display()))?;
    let stamp = unique_stamp()?;
    let candidate = parent.join(format!(
        ".mihomo-tui-migrate-{}-{stamp}.yaml",
        std::process::id()
    ));
    create_private_file(&candidate, content.as_bytes())?;
    let backup = parent.join(format!("config.yaml.{stamp}.wrapped.bak"));
    if let Err(error) = fs::copy(path, &backup) {
        let _ = fs::remove_file(&candidate);
        return Err(format!("无法备份旧包装配置：{error}"));
    }
    if let Err(error) = restrict_source_permissions(&backup) {
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_file(&backup);
        return Err(error);
    }
    if let Err(error) = fs::rename(&candidate, path) {
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_file(&backup);
        return Err(format!("无法迁移独立配置：{error}"));
    }
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步独立配置目录：{error}"))?;
    Ok(backup)
}

fn unique_stamp() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| error.to_string())
}

fn create_source(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("独立配置路径 {} 没有父目录", path.display()))?;
    let parent_existed = fs::symlink_metadata(parent).is_ok();
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建独立配置目录 {}：{error}", parent.display()))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("无法检查独立配置目录 {}：{error}", parent.display()))?;
    if !parent_metadata.file_type().is_dir() {
        return Err(format!(
            "独立配置目录 {} 必须是普通目录，不能是符号链接",
            parent.display()
        ));
    }
    #[cfg(unix)]
    if !parent_existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法设置独立配置目录权限 {}：{error}", parent.display()))?;
    }

    create_private_file(path, content)?;
    restrict_source_permissions(path)
}

fn create_private_file(path: &Path, content: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| format!("无法创建配置文件 {}：{error}", path.display()))?;
    if let Err(error) = file.write_all(content).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(format!("无法写入配置文件 {}：{error}", path.display()));
    }
    Ok(())
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
    fn imports_the_complete_legacy_mapping_as_the_native_config() {
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
        assert_eq!(document["secret"], "private");
        assert_eq!(document["experimental"]["custom-feature"]["enabled"], true);
        assert!(document.get("profile").is_none());
    }

    #[test]
    fn never_overwrites_an_existing_workspace() {
        let directory = TestDirectory::new("existing");
        let source = directory.join("config.yaml");
        let legacy = directory.join("legacy.yaml");
        let existing = "mixed-port: 17890\nrules: ['MATCH,DIRECT']\n";
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
    fn migrates_an_existing_wrapped_workspace_to_native_yaml() {
        let directory = TestDirectory::new("wrapped-migration");
        let source = directory.join("config.yaml");
        fs::write(
            &source,
            "kind: mihomo-tui/v1\nbackend: mihomo\nprofile:\n  mode: rule\n  custom:\n    keep: true\n  rules: ['MATCH,DIRECT']\n",
        )
        .unwrap();

        let result = initialize(&source, None).unwrap();

        let backup = match result {
            Initialization::MigratedWrapper(backup) => backup,
            other => panic!("unexpected initialization result: {other:?}"),
        };
        let document: Value = serde_yaml::from_str(&fs::read_to_string(&source).unwrap()).unwrap();
        assert_eq!(document["mode"], "rule");
        assert_eq!(document["custom"]["keep"], true);
        assert!(document.get("kind").is_none());
        assert!(
            fs::read_to_string(&backup)
                .unwrap()
                .contains("mihomo-tui/v1")
        );
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

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_as_the_workspace_source() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("source-symlink");
        let target = directory.join("target.yaml");
        let source = directory.join("config.yaml");
        fs::write(
            &target,
            "kind: mihomo-tui/v1\nbackend: mihomo\nprofile: {}\n",
        )
        .unwrap();
        symlink(&target, &source).unwrap();

        let error = initialize(&source, None).unwrap_err();

        assert!(error.contains("符号链接"), "unexpected error: {error}");
    }
}
