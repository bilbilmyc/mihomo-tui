use crate::{
    core::{CoreRelease, CoreVersion},
    system::{checked_output, clean_command, trusted_root_directory, trusted_root_file},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    fs::OpenOptions,
    io::{ErrorKind, Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink};

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

    pub(crate) fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cores: root.join("cores"),
            current: root.join("current"),
        }
    }

    pub fn binary(&self, version: CoreVersion) -> PathBuf {
        self.cores.join(version.to_string()).join("mihomo")
    }

    pub(crate) fn active_binary(&self) -> PathBuf {
        self.current.join("mihomo")
    }
}

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
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::time::{SystemTime, UNIX_EPOCH};

            static NEXT_TREE: AtomicU64 = AtomicU64::new(0);

            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = Path::new("/tmp").join(format!(
                "mihomo-tui-core-status-{}-{unique}-{}",
                std::process::id(),
                NEXT_TREE.fetch_add(1, Ordering::Relaxed)
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

    #[cfg(unix)]
    #[test]
    fn staging_a_version_never_overwrites_an_existing_core() {
        let tree = TestTree::create();
        let source = tree.write_core("source/mihomo", "v1.19.29");
        let paths = CorePaths::under(&tree.0.join("managed"));
        let version = CoreVersion::parse("v1.19.29").unwrap();

        install_version(&paths, &source, version).unwrap();
        let installed = std::fs::read(paths.binary(version)).unwrap();
        install_version(&paths, &source, version).unwrap();
        std::fs::write(
            &source,
            "#!/bin/sh\nprintf 'Mihomo Meta v1.19.29 changed\\n'\n",
        )
        .unwrap();
        let error = install_version(&paths, &source, version).unwrap_err();

        assert!(error.contains("different bytes"));
        assert_eq!(std::fs::read(paths.binary(version)).unwrap(), installed);
    }

    #[cfg(unix)]
    #[test]
    fn switching_current_uses_the_exact_relative_version_target() {
        let tree = TestTree::create();
        let paths = CorePaths::under(&tree.0.join("managed"));
        let source_28 = tree.write_core("source/v28", "v1.19.28");
        let source_29 = tree.write_core("source/v29", "v1.19.29");
        let v28 = CoreVersion::parse("v1.19.28").unwrap();
        let v29 = CoreVersion::parse("v1.19.29").unwrap();
        install_version(&paths, &source_28, v28).unwrap();
        install_version(&paths, &source_29, v29).unwrap();

        switch_current(&paths, v28).unwrap();
        switch_current(&paths, v29).unwrap();

        assert_eq!(
            std::fs::read_link(&paths.current).unwrap(),
            Path::new("cores/v1.19.29")
        );
        assert_eq!(inspect_active_version(&paths).unwrap(), Some(v29));
    }
}
