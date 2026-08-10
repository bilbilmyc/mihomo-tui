use crate::{
    core::{CoreRelease, CoreVersion},
    system::{checked_output, clean_command, trusted_root_directory, trusted_root_file},
};
use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

pub const MANAGED_ROOT: &str = "/usr/lib/mihomo-tui";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePaths {
    root: PathBuf,
    cores: PathBuf,
    current: PathBuf,
}

impl CorePaths {
    pub fn system() -> Self {
        Self::under(Path::new(MANAGED_ROOT))
    }

    fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cores: root.join("cores"),
            current: root.join("current"),
        }
    }

    pub fn binary(&self, version: CoreVersion) -> PathBuf {
        self.cores.join(version.to_string()).join("mihomo")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoreStatus {
    active: Option<CoreVersion>,
    installed: Vec<CoreVersion>,
    system: Vec<(PathBuf, CoreVersion)>,
    recommended: CoreVersion,
    minimum_supported: CoreVersion,
    maximum_exclusive: CoreVersion,
}

fn inspect_at(paths: &CorePaths, system_binaries: &[PathBuf]) -> Result<CoreStatus, String> {
    let release = CoreRelease::embedded()?;
    let installed = inspect_managed_versions(paths)?;
    let active = inspect_active_version(paths)?;
    if let Some(active_version) = active
        && installed.binary_search(&active_version).is_err()
    {
        return Err(format!(
            "managed current link selects {}, but that version is not installed",
            active_version
        ));
    }
    let mut system = Vec::new();
    for binary in system_binaries {
        match fs::symlink_metadata(binary) {
            Ok(_) if !trusted_root_file(binary) => {
                return Err(format!(
                    "system Mihomo binary {} is not a trusted root file",
                    binary.display()
                ));
            }
            Ok(_) => system.push((binary.clone(), binary_version(binary)?)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect system Mihomo binary {}: {error}",
                    binary.display()
                ));
            }
        }
    }
    Ok(CoreStatus {
        active,
        installed,
        system,
        recommended: release.recommended(),
        minimum_supported: release.minimum_supported(),
        maximum_exclusive: release.maximum_exclusive(),
    })
}

pub fn status() -> Result<String, String> {
    let paths = CorePaths::system();
    let system_binaries = [
        PathBuf::from("/usr/bin/mihomo"),
        PathBuf::from("/usr/local/bin/mihomo"),
    ];
    inspect_at(&paths, &system_binaries).map(|status| render_status(&status))
}

fn parse_active_link_target(target: &Path) -> Result<CoreVersion, String> {
    let components = target.components().collect::<Vec<_>>();
    let [Component::Normal(cores), Component::Normal(version)] = components.as_slice() else {
        return Err(format!(
            "managed current link has invalid target {}",
            target.display()
        ));
    };
    if *cores != "cores" {
        return Err(format!(
            "managed current link must target cores/<version>, not {}",
            target.display()
        ));
    }
    let version = version
        .to_str()
        .ok_or_else(|| "managed current link version is not UTF-8".to_string())?;
    CoreVersion::parse(version)
}

fn inspect_managed_versions(paths: &CorePaths) -> Result<Vec<CoreVersion>, String> {
    match fs::symlink_metadata(&paths.root) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "cannot inspect managed core root {}: {error}",
                paths.root.display()
            ));
        }
        Ok(_) if !trusted_root_directory(&paths.root) => {
            return Err(format!(
                "managed core root {} is not a trusted root directory",
                paths.root.display()
            ));
        }
        Ok(_) => {}
    }
    match fs::symlink_metadata(&paths.cores) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "cannot inspect managed cores directory {}: {error}",
                paths.cores.display()
            ));
        }
        Ok(_) if !trusted_root_directory(&paths.cores) => {
            return Err(format!(
                "managed cores path {} is not a trusted root directory",
                paths.cores.display()
            ));
        }
        Ok(_) => {}
    }

    let entries = fs::read_dir(&paths.cores).map_err(|error| {
        format!(
            "cannot read managed cores directory {}: {error}",
            paths.cores.display()
        )
    })?;
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot inspect managed core entry: {error}"))?;
        let directory = entry.path();
        if !trusted_root_directory(&directory) {
            return Err(format!(
                "managed core entry {} is not a trusted root directory",
                directory.display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "managed core version directory is not UTF-8".to_string())?;
        let version = CoreVersion::parse(&name)?;
        let binary = paths.binary(version);
        if !trusted_root_file(&binary) {
            return Err(format!(
                "managed core binary {} is not a trusted root file",
                binary.display()
            ));
        }
        let actual = binary_version(&binary)?;
        if actual != version {
            return Err(format!(
                "managed core directory {version} contains binary {actual}"
            ));
        }
        versions.push(version);
    }
    versions.sort_unstable();
    Ok(versions)
}

fn inspect_active_version(paths: &CorePaths) -> Result<Option<CoreVersion>, String> {
    match fs::symlink_metadata(&paths.current) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect managed current link {}: {error}",
            paths.current.display()
        )),
        Ok(metadata) if !metadata.file_type().is_symlink() => Err(format!(
            "managed current path {} is not a symbolic link",
            paths.current.display()
        )),
        Ok(_) => fs::read_link(&paths.current)
            .map_err(|error| format!("cannot read managed current link: {error}"))
            .and_then(|target| parse_active_link_target(&target))
            .map(Some),
    }
}

fn binary_version(binary: &Path) -> Result<CoreVersion, String> {
    let output = clean_command(binary).arg("-v").output().map_err(|error| {
        format!(
            "cannot inspect Mihomo version at {}: {error}",
            binary.display()
        )
    })?;
    let output = checked_output(&format!("{} -v", binary.display()), output)?;
    let output = String::from_utf8(output.stdout)
        .map_err(|_| format!("Mihomo version at {} is not UTF-8", binary.display()))?;
    CoreVersion::from_mihomo_output(&output)
}

fn render_status(status: &CoreStatus) -> String {
    let active = status
        .active
        .map(|version| version.to_string())
        .unwrap_or_else(|| "none".into());
    let installed = if status.installed.is_empty() {
        "none".into()
    } else {
        status
            .installed
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let system = if status.system.is_empty() {
        "none".into()
    } else {
        status
            .system
            .iter()
            .map(|(path, version)| format!("{version} ({})", path.display()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "managed root: {MANAGED_ROOT}\nactive: {active}\ninstalled: {installed}\nsystem: {}\nrecommended: {}\ncompatible: >={}, <{}",
        system, status.recommended, status.minimum_supported, status.maximum_exclusive
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[cfg(unix)]
    struct TestTree(PathBuf);

    #[cfg(unix)]
    impl TestTree {
        fn create() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};

            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = Path::new("/tmp").join(format!(
                "mihomo-tui-core-status-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write_core(&self, relative: &str, version: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                format!("#!/bin/sh\nprintf 'Mihomo Meta {version} linux amd64\\n'\n"),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    #[cfg(unix)]
    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn managed_paths_use_versioned_immutable_directories() {
        let paths = CorePaths::under(Path::new("/managed"));
        let version = CoreVersion::parse("v1.19.29").unwrap();

        assert_eq!(paths.cores, Path::new("/managed/cores"));
        assert_eq!(paths.current, Path::new("/managed/current"));
        assert_eq!(
            paths.binary(version),
            Path::new("/managed/cores/v1.19.29/mihomo")
        );
    }

    #[test]
    fn active_link_only_accepts_a_relative_version_directory() {
        assert_eq!(
            parse_active_link_target(Path::new("cores/v1.19.29"))
                .unwrap()
                .to_string(),
            "v1.19.29"
        );

        for unsafe_target in [
            "/usr/lib/mihomo-tui/cores/v1.19.29",
            "../cores/v1.19.29",
            "cores/v1.19.29/mihomo",
            "cores/not-a-version",
        ] {
            assert!(
                parse_active_link_target(Path::new(unsafe_target)).is_err(),
                "accepted {unsafe_target}"
            );
        }
    }

    #[test]
    fn status_output_reports_the_managed_core_contract() {
        let release = CoreRelease::embedded().unwrap();
        let status = CoreStatus {
            active: Some(CoreVersion::parse("v1.19.28").unwrap()),
            installed: vec![
                CoreVersion::parse("v1.19.28").unwrap(),
                CoreVersion::parse("v1.19.29").unwrap(),
            ],
            system: vec![(
                PathBuf::from("/usr/bin/mihomo"),
                CoreVersion::parse("v1.19.28").unwrap(),
            )],
            recommended: release.recommended(),
            minimum_supported: release.minimum_supported(),
            maximum_exclusive: release.maximum_exclusive(),
        };

        let output = render_status(&status);

        assert!(output.contains("active: v1.19.28"));
        assert!(output.contains("installed: v1.19.28, v1.19.29"));
        assert!(output.contains("recommended: v1.19.29"));
        assert!(output.contains("compatible: >=v1.19.28, <v1.20.0"));
        assert!(output.contains("system: v1.19.28 (/usr/bin/mihomo)"));
    }

    #[cfg(unix)]
    #[test]
    fn status_inspects_managed_and_system_binaries_without_mutating_them() {
        let tree = TestTree::create();
        tree.write_core("cores/v1.19.29/mihomo", "v1.19.29");
        tree.write_core("cores/v1.19.28/mihomo", "v1.19.28");
        let system_binary = tree.write_core("system/mihomo", "v1.19.27");
        symlink("cores/v1.19.28", tree.0.join("current")).unwrap();

        let status = inspect_at(
            &CorePaths::under(&tree.0),
            std::slice::from_ref(&system_binary),
        )
        .unwrap();

        assert_eq!(
            status.installed,
            vec![
                CoreVersion::parse("v1.19.28").unwrap(),
                CoreVersion::parse("v1.19.29").unwrap()
            ]
        );
        assert_eq!(status.active, Some(CoreVersion::parse("v1.19.28").unwrap()));
        assert_eq!(
            status.system,
            vec![(system_binary, CoreVersion::parse("v1.19.27").unwrap())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn status_rejects_a_version_directory_with_a_different_binary() {
        let tree = TestTree::create();
        tree.write_core("cores/v1.19.29/mihomo", "v1.19.28");

        let error = inspect_at(&CorePaths::under(&tree.0), &[]).unwrap_err();

        assert!(error.contains("v1.19.29"));
        assert!(error.contains("v1.19.28"));
    }
}
