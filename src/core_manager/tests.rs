use super::{
    CorePaths,
    inventory::{
        CoreStatus, inspect_active_version, inspect_at, parse_active_link_target, render_status,
    },
    storage::{install_version, switch_current},
};
use crate::core::{CoreRelease, CoreVersion};
use std::path::{Path, PathBuf};

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
        use std::io::Write;

        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let staging = path.with_extension("writing");
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .unwrap();
            file.write_all(
                format!("#!/bin/sh\nprintf 'Mihomo Meta {version} linux amd64\\n'\n").as_bytes(),
            )
            .unwrap();
            file.sync_all().unwrap();
        }
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(staging, &path).unwrap();
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
        license_spdx: release.license().spdx.clone(),
    };

    let output = render_status(&status);

    assert!(output.contains("active: v1.19.28"));
    assert!(output.contains("installed: v1.19.28, v1.19.29"));
    assert!(output.contains("recommended: v1.19.29"));
    assert!(output.contains("compatible: >=v1.19.28, <v1.20.0"));
    assert!(output.contains("system: v1.19.28 (/usr/bin/mihomo)"));
    assert!(output.contains("license: GPL-3.0"));
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
