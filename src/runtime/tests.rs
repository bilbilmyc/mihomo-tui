use super::{
    Action, Inventory, RuntimeMode,
    install::{VersionRequirement, mihomo_binary_path_at, validate_installed_version_output},
    plan,
    systemd::{
        MANAGED_DROP_IN, UnitExpectation, config_apply_command, core_upgrade_restart_command,
        parse_systemd_unit, validate_systemd_unit_state, workspace_drop_in_content,
    },
};
use crate::{
    core::CoreVersion,
    core_manager::{CorePaths, install_version, switch_current},
};
use std::{fs, path::Path};

#[test]
fn managed_runtime_accepts_only_tested_core_versions() {
    for version in ["v1.19.28", "v1.19.29"] {
        let output = format!("Mihomo Meta {version} linux amd64 with go1.26.5");
        assert!(validate_installed_version_output(&output, VersionRequirement::Supported).is_ok());
    }

    for version in ["v1.19.27", "v1.20.0"] {
        let output = format!("Mihomo Meta {version} linux amd64 with go1.26.5");
        assert!(validate_installed_version_output(&output, VersionRequirement::Supported).is_err());
    }
}

#[test]
fn newly_installed_core_must_match_the_recommended_version() {
    assert!(
        validate_installed_version_output(
            "Mihomo Meta v1.19.29 linux amd64",
            VersionRequirement::Recommended,
        )
        .is_ok()
    );
    assert!(
        validate_installed_version_output(
            "Mihomo Meta v1.19.28 linux amd64",
            VersionRequirement::Recommended,
        )
        .is_err()
    );
}

#[test]
fn external_controller_never_manages_the_local_runtime() {
    let inventory = Inventory {
        binary: false,
        unit: false,
    };

    assert_eq!(plan(RuntimeMode::External, inventory, true), Action::None);
}

#[test]
fn clean_local_machine_is_installed_when_auto_install_is_enabled() {
    let inventory = Inventory {
        binary: false,
        unit: false,
    };

    assert_eq!(
        plan(RuntimeMode::ManagedLocal, inventory, true),
        Action::Install
    );
}

#[test]
fn clean_local_machine_stays_untouched_when_auto_install_is_disabled() {
    let inventory = Inventory {
        binary: false,
        unit: false,
    };

    assert_eq!(
        plan(RuntimeMode::ManagedLocal, inventory, false),
        Action::None
    );
}

#[test]
fn complete_local_install_is_prepared_without_downloading() {
    let inventory = Inventory {
        binary: true,
        unit: true,
    };

    assert_eq!(
        plan(RuntimeMode::ManagedLocal, inventory, true),
        Action::Prepare
    );
}

#[test]
fn partial_install_is_not_overwritten() {
    for inventory in [
        Inventory {
            binary: true,
            unit: false,
        },
        Inventory {
            binary: false,
            unit: true,
        },
    ] {
        assert_eq!(
            plan(RuntimeMode::ManagedLocal, inventory, true),
            Action::RejectPartialInstall
        );
    }
}

#[test]
fn applying_an_edited_config_starts_an_inactive_service() {
    let (description, arguments) = config_apply_command();

    assert_eq!(description, "systemctl reload-or-restart mihomo.service");
    assert_eq!(arguments, ["reload-or-restart", "mihomo.service"]);
}

#[test]
fn core_upgrade_uses_a_full_service_restart() {
    let (description, arguments) = core_upgrade_restart_command();

    assert_eq!(description, "systemctl restart mihomo.service");
    assert_eq!(arguments, ["restart", "mihomo.service"]);
}

#[test]
fn systemd_units_outside_the_managed_paths_are_rejected() {
    let hidden = parse_systemd_unit(
        "LoadState=loaded\nFragmentPath=/run/systemd/system/mihomo.service\nDropInPaths=\n",
    )
    .unwrap();
    assert!(validate_systemd_unit_state(&hidden, UnitExpectation::Packaged).is_err());

    let packaged = parse_systemd_unit(
        "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths=\n",
    )
    .unwrap();
    assert!(validate_systemd_unit_state(&packaged, UnitExpectation::Packaged).is_ok());

    let overridden = parse_systemd_unit(
        "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths=/run/systemd/system.control/mihomo.service.d/50-CPUQuota.conf\n",
    )
    .unwrap();
    assert!(validate_systemd_unit_state(&overridden, UnitExpectation::Packaged).is_err());

    let managed = parse_systemd_unit(&format!(
        "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths={MANAGED_DROP_IN}\n"
    ))
    .unwrap();
    assert!(validate_systemd_unit_state(&managed, UnitExpectation::Packaged).is_ok());

    let absent = parse_systemd_unit("LoadState=not-found\nFragmentPath=\nDropInPaths=\n").unwrap();
    assert!(validate_systemd_unit_state(&absent, UnitExpectation::Absent).is_ok());
}

#[test]
fn managed_drop_in_points_mihomo_at_the_single_config_directory() {
    assert_eq!(
        workspace_drop_in_content(Path::new("/usr/bin/mihomo")),
        "[Service]\nExecStart=\nExecStart=/usr/bin/mihomo -d /etc/mihomo-tui\n"
    );
}

#[cfg(unix)]
#[test]
fn bundled_managed_core_is_selected_without_a_legacy_binary() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Path::new("/tmp").join(format!(
        "mihomo-tui-runtime-bundle-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let source = root.join("mihomo");
    fs::write(
        &source,
        "#!/bin/sh\nprintf 'Mihomo Meta v1.19.29 linux amd64\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let paths = CorePaths::under(&root.join("managed"));
    let version = CoreVersion::parse("v1.19.29").unwrap();
    install_version(&paths, &source, version).unwrap();
    switch_current(&paths, version).unwrap();

    let selected = mihomo_binary_path_at(&paths, &[]).unwrap();

    assert_eq!(selected, Some(paths.binary(version)));
    fs::remove_dir_all(root).unwrap();
}
