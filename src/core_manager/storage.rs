use super::{CorePaths, inventory::binary_version, inventory::parse_active_link_target};
use crate::{core::CoreVersion, system::trusted_root_directory, system::trusted_root_file};
use sha2::{Digest, Sha256};
use std::{
    fs,
    fs::OpenOptions,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink};

struct StagingDirectory {
    path: PathBuf,
}

impl StagingDirectory {
    #[cfg(unix)]
    fn create(paths: &CorePaths, version: CoreVersion) -> Result<Self, String> {
        let path = paths
            .root
            .join(format!(".stage-{version}-{}", unique_suffix()?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| format!("cannot create core staging directory: {error}"))?;
        Ok(Self { path })
    }

    #[cfg(not(unix))]
    fn create(_paths: &CorePaths, _version: CoreVersion) -> Result<Self, String> {
        Err("managed core staging is only supported on Unix".into())
    }

    fn persist(mut self, final_path: &Path) -> Result<(), String> {
        fs::rename(&self.path, final_path)
            .map_err(|error| format!("cannot install managed core version: {error}"))?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(crate) fn install_version(
    paths: &CorePaths,
    source: &Path,
    expected: CoreVersion,
) -> Result<(), String> {
    if !trusted_root_file(source) {
        return Err(format!(
            "Mihomo source {} is not a trusted root file",
            source.display()
        ));
    }
    if binary_version(source)? != expected {
        return Err(format!(
            "Mihomo source {} does not contain {expected}",
            source.display()
        ));
    }
    let source_sha256 = file_sha256(source)?;
    ensure_layout(paths)?;
    let final_directory = paths.cores.join(expected.to_string());
    match fs::symlink_metadata(&final_directory) {
        Ok(_) => {
            if !trusted_root_directory(&final_directory)
                || !trusted_root_file(&paths.binary(expected))
                || binary_version(&paths.binary(expected))? != expected
            {
                return Err(format!(
                    "existing managed core directory {} is invalid",
                    final_directory.display()
                ));
            }
            if file_sha256(&paths.binary(expected))? != source_sha256 {
                return Err(format!(
                    "existing managed core {expected} has different bytes and will not be overwritten"
                ));
            }
            return Ok(());
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect managed core destination {}: {error}",
                final_directory.display()
            ));
        }
    }

    let staging = StagingDirectory::create(paths, expected)?;
    let staged_binary = staging.path.join("mihomo");
    let mut input = fs::File::open(source)
        .map_err(|error| format!("cannot open Mihomo source {}: {error}", source.display()))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o755);
    let mut output = options.open(&staged_binary).map_err(|error| {
        format!(
            "cannot create staged Mihomo binary {}: {error}",
            staged_binary.display()
        )
    })?;
    std::io::copy(&mut input, &mut output)
        .map_err(|error| format!("cannot copy staged Mihomo binary: {error}"))?;
    output
        .flush()
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("cannot sync staged Mihomo binary: {error}"))?;
    drop(output);
    #[cfg(unix)]
    {
        fs::set_permissions(&staged_binary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot set staged Mihomo permissions: {error}"))?;
        fs::set_permissions(&staging.path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("cannot set managed version permissions: {error}"))?;
    }
    if source_sha256 != file_sha256(&staged_binary)? {
        return Err("staged Mihomo binary does not match its validated source".into());
    }
    staging.persist(&final_directory)?;
    sync_directory(&paths.cores)
}

pub(crate) fn switch_current(paths: &CorePaths, version: CoreVersion) -> Result<(), String> {
    ensure_layout(paths)?;
    let binary = paths.binary(version);
    if !trusted_root_file(&binary) || binary_version(&binary)? != version {
        return Err(format!("cannot activate invalid managed core {version}"));
    }
    match fs::symlink_metadata(&paths.current) {
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            return Err(format!(
                "managed current path {} is not a symbolic link",
                paths.current.display()
            ));
        }
        Ok(_) => {
            let target = fs::read_link(&paths.current)
                .map_err(|error| format!("cannot read managed current link: {error}"))?;
            parse_active_link_target(&target)?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect managed current link: {error}")),
    }

    #[cfg(unix)]
    {
        let candidate = paths.root.join(format!(".current-{}", unique_suffix()?));
        let target = PathBuf::from("cores").join(version.to_string());
        symlink(&target, &candidate)
            .map_err(|error| format!("cannot create managed current candidate: {error}"))?;
        if let Err(error) = fs::rename(&candidate, &paths.current) {
            let _ = fs::remove_file(&candidate);
            return Err(format!("cannot switch managed current link: {error}"));
        }
        sync_directory(&paths.root)
    }
    #[cfg(not(unix))]
    {
        Err("managed core activation is only supported on Unix".into())
    }
}

fn ensure_layout(paths: &CorePaths) -> Result<(), String> {
    ensure_trusted_directory(&paths.root)?;
    ensure_trusted_directory(&paths.cores)
}

fn ensure_trusted_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) if trusted_root_directory(path) => return Ok(()),
        Ok(_) => {
            return Err(format!(
                "managed core path {} is not a trusted root directory",
                path.display()
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect managed core path {}: {error}",
                path.display()
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("managed core path {} has no parent", path.display()))?;
    if !trusted_root_directory(parent) {
        return Err(format!(
            "managed core parent {} is not a trusted root directory",
            parent.display()
        ));
    }
    #[cfg(unix)]
    fs::DirBuilder::new()
        .mode(0o755)
        .create(path)
        .map_err(|error| {
            format!(
                "cannot create managed core path {}: {error}",
                path.display()
            )
        })?;
    #[cfg(not(unix))]
    return Err("managed core layout is only supported on Unix".into());
    if !trusted_root_directory(path) {
        return Err(format!(
            "new managed core path {} is not a trusted root directory",
            path.display()
        ));
    }
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot sync directory {}: {error}", path.display()))
}

fn file_sha256(path: &Path) -> Result<[u8; 32], String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("cannot hash file {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash file {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn unique_suffix() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_nanos();
    Ok(format!("{}-{nanos}", std::process::id()))
}
