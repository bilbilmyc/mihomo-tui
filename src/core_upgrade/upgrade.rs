use super::activation::{HostActivation, activate_with_rollback};
use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::{CorePaths, binary_version, install_version, managed_active_version},
    core_package,
    mihomo::MihomoClient,
    runtime,
    system::{
        acquire_runtime_lock, checked_output, clean_command, effective_root,
        trusted_root_directory, trusted_root_file,
    },
    workspace,
};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const SYSTEM_BINARIES: [&str; 2] = ["/usr/bin/mihomo", "/usr/local/bin/mihomo"];

pub(super) fn upgrade() -> Result<String, String> {
    if !effective_root() {
        return Err(runtime::root_required_message().into());
    }
    let _lock = acquire_runtime_lock()?;
    let release = CoreRelease::embedded()?;
    let candidate_version = release.recommended();
    let paths = CorePaths::system();
    let active = managed_active_version(&paths)?;
    if active == Some(candidate_version) {
        return Ok(format!(
            "managed Mihomo core {candidate_version} is already up to date"
        ));
    }

    core_package::ensure_debian_host()?;
    runtime::validate_upgrade_service()?;
    validate_owned_config()?;
    let connection = crate::discovery::discover(Path::new(workspace::DEFAULT_SOURCE_PATH))
        .ok_or_else(|| {
            format!(
                "{} must define external-controller before a core upgrade",
                workspace::DEFAULT_SOURCE_PATH
            )
        })?;
    let Some(previous) = prepare_previous_core(&paths, &release, active)? else {
        return Ok(format!(
            "current Mihomo core {candidate_version} already matches the recommended version; no managed activation was needed"
        ));
    };

    if let Some(candidate) = installed_candidate(&paths, candidate_version)? {
        eprintln!("validating bundled Mihomo {candidate_version}...");
        validate_candidate(&candidate, candidate_version)?;
    } else {
        let package = release.package_for(std::env::consts::OS, std::env::consts::ARCH)?;
        eprintln!("downloading and verifying official Mihomo {candidate_version}...");
        let downloaded = core_package::download_package(&release, package)?;
        let extracted = core_package::extract_core(&downloaded)?;
        validate_candidate(extracted.candidate(), candidate_version)?;
        install_version(&paths, extracted.candidate(), candidate_version)?;
    }

    let client = MihomoClient::new(connection.controller, connection.secret)
        .map_err(|error| format!("cannot create Mihomo health client: {error}"))?;
    let mut activation = HostActivation { paths, client };
    activate_with_rollback(&mut activation, previous, candidate_version)?;
    Ok(format!(
        "managed Mihomo core upgraded from {previous} to {candidate_version}"
    ))
}

pub(super) fn installed_candidate(
    paths: &CorePaths,
    expected: CoreVersion,
) -> Result<Option<PathBuf>, String> {
    managed_active_version(paths)?;
    let binary = paths.binary(expected);
    match fs::symlink_metadata(&binary) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect installed managed core {}: {error}",
            binary.display()
        )),
        Ok(_) if !trusted_root_file(&binary) => Err(format!(
            "installed managed core {} is not a trusted root file",
            binary.display()
        )),
        Ok(_) if binary_version(&binary)? != expected => Err(format!(
            "installed managed core {} does not contain {expected}",
            binary.display()
        )),
        Ok(_) => Ok(Some(binary)),
    }
}

fn prepare_previous_core(
    paths: &CorePaths,
    release: &CoreRelease,
    active: Option<CoreVersion>,
) -> Result<Option<CoreVersion>, String> {
    if let Some(version) = active {
        return rollback_version(release, version);
    }

    let (binary, version) = find_system_core()?;
    let rollback = rollback_version(release, version)?;
    if rollback.is_none() {
        return Ok(None);
    }
    install_version(paths, &binary, version)?;
    Ok(rollback)
}

pub(super) fn rollback_version(
    release: &CoreRelease,
    version: CoreVersion,
) -> Result<Option<CoreVersion>, String> {
    release.require_supported(version)?;
    Ok((version != release.recommended()).then_some(version))
}

fn find_system_core() -> Result<(PathBuf, CoreVersion), String> {
    for candidate in SYSTEM_BINARIES.map(PathBuf::from) {
        match fs::symlink_metadata(&candidate) {
            Ok(_) if !trusted_root_file(&candidate) => {
                return Err(format!(
                    "system Mihomo binary {} is not a trusted root file",
                    candidate.display()
                ));
            }
            Ok(_) => return Ok((candidate.clone(), binary_version(&candidate)?)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect system Mihomo binary {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Err("no trusted current Mihomo core is available for rollback".into())
}

fn validate_owned_config() -> Result<(), String> {
    let config = Path::new(workspace::DEFAULT_SOURCE_PATH);
    let data_dir = config
        .parent()
        .ok_or_else(|| "owned Mihomo config has no data directory".to_string())?;
    if !trusted_root_directory(data_dir) {
        return Err(format!(
            "owned Mihomo data directory {} is not a trusted root directory",
            data_dir.display()
        ));
    }
    if !trusted_root_file(config) {
        return Err(format!(
            "owned Mihomo config {} is not a trusted root file",
            config.display()
        ));
    }
    Ok(())
}

fn validate_candidate(candidate: &Path, expected: CoreVersion) -> Result<(), String> {
    let actual = binary_version(candidate)?;
    if actual != expected {
        return Err(format!(
            "extracted Mihomo version mismatch: expected {expected}, received {actual}"
        ));
    }
    let data_dir = Path::new(workspace::DEFAULT_SOURCE_PATH)
        .parent()
        .ok_or_else(|| "owned Mihomo config has no data directory".to_string())?;
    let output = clean_command(candidate)
        .args(["-t", "-d"])
        .arg(data_dir)
        .output()
        .map_err(|error| format!("cannot validate config with candidate Mihomo: {error}"))?;
    checked_output("candidate Mihomo config validation", output).map(|_| ())
}
