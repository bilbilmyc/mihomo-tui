use super::{CorePaths, MANAGED_ROOT};
use crate::{
    core::{CoreRelease, CoreVersion},
    system::{checked_output, clean_command, trusted_root_directory, trusted_root_file},
};
use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};

const VERSION_BUSY_RETRIES: usize = 4;
const VERSION_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CoreStatus {
    pub(super) active: Option<CoreVersion>,
    pub(super) installed: Vec<CoreVersion>,
    pub(super) system: Vec<(PathBuf, CoreVersion)>,
    pub(super) recommended: CoreVersion,
    pub(super) minimum_supported: CoreVersion,
    pub(super) maximum_exclusive: CoreVersion,
    pub(super) license_spdx: String,
}

pub(super) fn inspect_at(
    paths: &CorePaths,
    system_binaries: &[PathBuf],
) -> Result<CoreStatus, String> {
    let release = CoreRelease::embedded()?;
    let installed = inspect_managed_versions(paths)?;
    let active = validate_active_version(paths, &installed)?;
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
        license_spdx: release.license().spdx.clone(),
    })
}

pub(crate) fn managed_active_version(paths: &CorePaths) -> Result<Option<CoreVersion>, String> {
    let installed = inspect_managed_versions(paths)?;
    validate_active_version(paths, &installed)
}

fn validate_active_version(
    paths: &CorePaths,
    installed: &[CoreVersion],
) -> Result<Option<CoreVersion>, String> {
    let active = inspect_active_version(paths)?;
    if let Some(active_version) = active
        && installed.binary_search(&active_version).is_err()
    {
        return Err(format!(
            "managed current link selects {}, but that version is not installed",
            active_version
        ));
    }
    Ok(active)
}

pub(super) fn status() -> Result<String, String> {
    let paths = CorePaths::system();
    let system_binaries = [
        PathBuf::from("/usr/bin/mihomo"),
        PathBuf::from("/usr/local/bin/mihomo"),
    ];
    inspect_at(&paths, &system_binaries).map(|status| render_status(&status))
}

pub(super) fn parse_active_link_target(target: &Path) -> Result<CoreVersion, String> {
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

pub(crate) fn inspect_active_version(paths: &CorePaths) -> Result<Option<CoreVersion>, String> {
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

pub(crate) fn binary_version(binary: &Path) -> Result<CoreVersion, String> {
    let mut busy_retries = VERSION_BUSY_RETRIES;
    let output = loop {
        match clean_command(binary).arg("-v").output() {
            Ok(output) => break output,
            Err(error) if error.kind() == ErrorKind::ExecutableFileBusy && busy_retries > 0 => {
                busy_retries -= 1;
                thread::sleep(VERSION_BUSY_RETRY_DELAY);
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect Mihomo version at {}: {error}",
                    binary.display()
                ));
            }
        }
    };
    let output = checked_output(&format!("{} -v", binary.display()), output)?;
    let output = String::from_utf8(output.stdout)
        .map_err(|_| format!("Mihomo version at {} is not UTF-8", binary.display()))?;
    CoreVersion::from_mihomo_output(&output)
}

pub(super) fn render_status(status: &CoreStatus) -> String {
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
        "managed root: {MANAGED_ROOT}\nactive: {active}\ninstalled: {installed}\nsystem: {}\nrecommended: {}\ncompatible: >={}, <{}\nlicense: {}",
        system,
        status.recommended,
        status.minimum_supported,
        status.maximum_exclusive,
        status.license_spdx
    )
}
