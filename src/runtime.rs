use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::{CorePaths, managed_active_version},
    core_package,
    system::{
        acquire_runtime_lock, checked_output, clean_command, effective_root, run_privileged,
        trusted_root_file,
    },
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Output,
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

const BINARY_PATHS: [&str; 2] = ["/usr/bin/mihomo", "/usr/local/bin/mihomo"];
const UNIT_PATHS: [&str; 3] = [
    "/etc/systemd/system/mihomo.service",
    "/usr/lib/systemd/system/mihomo.service",
    "/lib/systemd/system/mihomo.service",
];
const MANAGED_DROP_IN: &str = "/etc/systemd/system/mihomo.service.d/10-mihomo-tui.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    External,
    ManagedLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inventory {
    pub binary: bool,
    pub unit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Prepare,
    Install,
    RejectPartialInstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitExpectation {
    Absent,
    Packaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemdUnitState {
    load_state: String,
    fragment_path: Option<PathBuf>,
    drop_in_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionRequirement {
    Supported,
    Recommended,
}

pub fn plan(mode: RuntimeMode, inventory: Inventory, auto_install: bool) -> Action {
    if mode == RuntimeMode::External {
        return Action::None;
    }
    match (inventory.binary, inventory.unit) {
        (true, true) => Action::Prepare,
        (false, false) if auto_install => Action::Install,
        (false, false) => Action::None,
        _ => Action::RejectPartialInstall,
    }
}

pub fn ensure(mode: RuntimeMode, auto_install: bool) -> Result<(), String> {
    if mode == RuntimeMode::External {
        return Ok(());
    }
    let inventory = inspect()?;
    match plan(mode, inventory, auto_install) {
        Action::None => Ok(()),
        Action::RejectPartialInstall => Err(partial_install_error(inventory)),
        Action::Prepare | Action::Install => {
            if !effective_root() {
                return Err(root_required_message().into());
            }
            let _lock = acquire_runtime_lock()?;
            let inventory = inspect()?;
            match plan(mode, inventory, auto_install) {
                Action::None => Ok(()),
                Action::Prepare => prepare_existing_for_apply(),
                Action::Install => install_for_apply(),
                Action::RejectPartialInstall => Err(partial_install_error(inventory)),
            }
        }
    }
}

pub(crate) fn root_required_message() -> &'static str {
    "权限不足：自动安装或管理本机 Mihomo 需要 root 权限。\n请在原命令前添加 sudo 重新运行，例如：sudo mihomo-tui\n如果只连接已有实例，请使用 --controller。"
}

fn partial_install_error(inventory: Inventory) -> String {
    format!(
        "检测到不完整的 Mihomo 安装（binary={}，service={}），为避免覆盖现有文件已停止；请修复后重试或使用 --controller",
        inventory.binary, inventory.unit
    )
}

fn prepare_existing_for_apply() -> Result<(), String> {
    validate_existing_install()?;
    validate_installed_core(VersionRequirement::Supported)?;
    validate_systemd_unit(UnitExpectation::Packaged)
}

fn validate_installed_core(requirement: VersionRequirement) -> Result<CoreVersion, String> {
    let binary = mihomo_binary_path()?
        .ok_or_else(|| "未在受信任路径中找到 Mihomo（/usr/bin 或 /usr/local/bin）".to_string())?;
    validate_installed_core_at(&binary, requirement)
}

fn validate_installed_core_at(
    binary: &Path,
    requirement: VersionRequirement,
) -> Result<CoreVersion, String> {
    let output = clean_command(binary)
        .arg("-v")
        .output()
        .map_err(|error| format!("无法检查 Mihomo 版本：{error}"))?;
    let output = checked_output("mihomo -v", output)?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "Mihomo 版本信息不是有效 UTF-8".to_string())?;
    validate_installed_version_output(&stdout, requirement)
}

fn validate_installed_version_output(
    output: &str,
    requirement: VersionRequirement,
) -> Result<CoreVersion, String> {
    let release = CoreRelease::embedded()?;
    let version = CoreVersion::from_mihomo_output(output)?;
    match requirement {
        VersionRequirement::Supported => release.require_supported(version)?,
        VersionRequirement::Recommended if version != release.recommended() => {
            return Err(format!(
                "安装后的 Mihomo 版本不匹配：期望 {}，实际 {version}",
                release.recommended()
            ));
        }
        VersionRequirement::Recommended => {}
    }
    Ok(version)
}

pub fn reload_service() -> Result<(), String> {
    ensure_systemd()?;
    validate_systemd_unit(UnitExpectation::Packaged)?;
    let systemctl = systemctl_path()?;
    let (description, arguments) = config_apply_command();
    let output = clean_command(&systemctl)
        .args(arguments)
        .output()
        .map_err(|error| format!("无法执行 {description}：{error}"))?;
    checked_output(description, output).map(|_| ())
}

fn config_apply_command() -> (&'static str, [&'static str; 2]) {
    (
        "systemctl reload-or-restart mihomo.service",
        ["reload-or-restart", "mihomo.service"],
    )
}

pub fn validate_config(candidate: &Path, data_dir: &Path) -> Result<Output, String> {
    let binary = mihomo_binary_path()?
        .ok_or_else(|| "未在受信任路径中找到 Mihomo（/usr/bin 或 /usr/local/bin）".to_string())?;
    clean_command(&binary)
        .args(["-t", "-f"])
        .arg(candidate)
        .arg("-d")
        .arg(data_dir)
        .output()
        .map_err(|error| format!("无法执行 Mihomo 配置校验：{error}"))
}

pub fn configure_workspace_service(config_path: &Path) -> Result<(), String> {
    let binary = mihomo_binary_path()?.ok_or_else(|| "找不到受信任的 Mihomo".to_string())?;
    configure_workspace_service_with_binary(config_path, &binary)
}

pub(crate) fn validate_upgrade_service() -> Result<(), String> {
    if !effective_root() {
        return Err(root_required_message().into());
    }
    validate_systemd_unit(UnitExpectation::Packaged)
}

pub(crate) fn configure_managed_core_service() -> Result<(), String> {
    let binary = CorePaths::system().active_binary();
    if !trusted_root_file(&binary) {
        return Err(format!("找不到受信任的托管 Mihomo {}", binary.display()));
    }
    configure_workspace_service_with_binary(
        Path::new(crate::workspace::DEFAULT_SOURCE_PATH),
        &binary,
    )
}

fn configure_workspace_service_with_binary(
    config_path: &Path,
    binary: &Path,
) -> Result<(), String> {
    if config_path != Path::new(crate::workspace::DEFAULT_SOURCE_PATH) {
        return Err(format!(
            "本机托管模式只支持 {}",
            crate::workspace::DEFAULT_SOURCE_PATH
        ));
    }
    if !effective_root() {
        return Err(root_required_message().into());
    }
    validate_systemd_unit(UnitExpectation::Packaged)?;
    let drop_in = Path::new(MANAGED_DROP_IN);
    let directory = drop_in
        .parent()
        .ok_or_else(|| "systemd drop-in 路径没有父目录".to_string())?;
    let existed = fs::symlink_metadata(directory).is_ok();
    fs::create_dir_all(directory)
        .map_err(|error| format!("无法创建 Mihomo systemd drop-in 目录：{error}"))?;
    #[cfg(unix)]
    if !existed {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("无法设置 Mihomo systemd drop-in 目录权限：{error}"))?;
    }
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("无法检查 Mihomo systemd drop-in 目录：{error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("{} 不是普通目录", directory.display()));
    }
    #[cfg(unix)]
    {
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!("{} 的所有者或权限不安全", directory.display()));
        }
    }
    let content = workspace_drop_in_content(binary);
    if fs::symlink_metadata(drop_in).is_ok() && !trusted_root_file(drop_in) {
        return Err(format!("{} 不是安全的 root 普通文件", drop_in.display()));
    }
    if fs::read_to_string(drop_in).ok().as_deref() == Some(content.as_str()) {
        return daemon_reload();
    }
    let candidate = directory.join(format!(".10-mihomo-tui-{}.conf", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o644);
    let mut file = options
        .open(&candidate)
        .map_err(|error| format!("无法创建 Mihomo systemd drop-in 候选：{error}"))?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&candidate);
        return Err(format!("无法写入 Mihomo systemd drop-in：{error}"));
    }
    if let Err(error) = fs::rename(&candidate, drop_in) {
        let _ = fs::remove_file(&candidate);
        return Err(format!("无法安装 Mihomo systemd drop-in：{error}"));
    }
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步 Mihomo systemd drop-in 目录：{error}"))?;
    daemon_reload()
}

pub(crate) fn restart_service() -> Result<(), String> {
    ensure_systemd()?;
    validate_systemd_unit(UnitExpectation::Packaged)?;
    let systemctl = systemctl_path()?;
    let (description, arguments) = core_upgrade_restart_command();
    let output = run_privileged(&systemctl, &arguments)?;
    checked_output(description, output).map(|_| ())
}

fn core_upgrade_restart_command() -> (&'static str, [&'static str; 2]) {
    (
        "systemctl restart mihomo.service",
        ["restart", "mihomo.service"],
    )
}

fn workspace_drop_in_content(binary: &Path) -> String {
    format!(
        "[Service]\nExecStart=\nExecStart={} -d /etc/mihomo-tui\n",
        binary.display()
    )
}

fn inspect() -> Result<Inventory, String> {
    Ok(Inventory {
        binary: mihomo_binary_path()?.is_some(),
        unit: UNIT_PATHS.iter().any(path_present),
    })
}

fn path_present(path: impl AsRef<Path>) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn validate_existing_install() -> Result<(), String> {
    for (kind, paths) in [
        ("binary", BINARY_PATHS.as_slice()),
        ("service", UNIT_PATHS.as_slice()),
    ] {
        for path in paths.iter().filter(|path| path_present(path)) {
            if !trusted_root_file(Path::new(path)) {
                return Err(format!(
                    "Mihomo {kind} 文件 {path} 不是安全的 root 普通文件，拒绝启动"
                ));
            }
        }
    }
    Ok(())
}

fn mihomo_binary_path() -> Result<Option<PathBuf>, String> {
    mihomo_binary_path_at(
        &CorePaths::system(),
        &BINARY_PATHS.iter().map(PathBuf::from).collect::<Vec<_>>(),
    )
}

fn mihomo_binary_path_at(
    managed_paths: &CorePaths,
    legacy_paths: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    if let Some(version) = managed_active_version(managed_paths)? {
        return Ok(Some(managed_paths.binary(version)));
    }
    Ok(legacy_paths
        .iter()
        .find(|path| trusted_root_file(path))
        .cloned())
}

fn systemctl_path() -> Result<PathBuf, String> {
    ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| trusted_root_file(path))
        .ok_or_else(|| "找不到受信任的 systemctl 可执行文件".to_string())
}

fn parse_systemd_unit(output: &str) -> Result<SystemdUnitState, String> {
    let mut load_state = None;
    let mut fragment_path = None;
    let mut drop_in_paths = None;
    for line in output.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "LoadState" => load_state = Some(value.trim().to_string()),
            "FragmentPath" => {
                fragment_path =
                    Some((!value.trim().is_empty()).then(|| PathBuf::from(value.trim())))
            }
            "DropInPaths" => {
                drop_in_paths = Some(value.split_whitespace().map(PathBuf::from).collect())
            }
            _ => {}
        }
    }
    Ok(SystemdUnitState {
        load_state: load_state
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 LoadState".to_string())?,
        fragment_path: fragment_path
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 FragmentPath".to_string())?,
        drop_in_paths: drop_in_paths
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 DropInPaths".to_string())?,
    })
}

fn validate_systemd_unit_state(
    state: &SystemdUnitState,
    expectation: UnitExpectation,
) -> Result<(), String> {
    let absent = state.load_state == "not-found"
        && state.fragment_path.is_none()
        && state.drop_in_paths.is_empty();
    let managed_drop_ins = state.drop_in_paths.is_empty()
        || state.drop_in_paths.as_slice() == [PathBuf::from(MANAGED_DROP_IN)];
    let packaged = state.load_state == "loaded"
        && managed_drop_ins
        && state.fragment_path.as_deref().is_some_and(|path| {
            UNIT_PATHS
                .iter()
                .any(|expected| path == Path::new(expected))
        });
    let valid = match expectation {
        UnitExpectation::Absent => absent,
        UnitExpectation::Packaged => packaged,
    };
    if valid {
        return Ok(());
    }
    Err(format!(
        "systemd 中存在未受管理的 mihomo.service（LoadState={}，FragmentPath={}，DropIns={}），拒绝继续",
        state.load_state,
        state
            .fragment_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".into()),
        state
            .drop_in_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

fn validate_systemd_unit(expectation: UnitExpectation) -> Result<(), String> {
    ensure_systemd()?;
    let systemctl = systemctl_path()?;
    let output = clean_command(&systemctl)
        .args([
            "show",
            "mihomo.service",
            "--property=LoadState",
            "--property=FragmentPath",
            "--property=DropInPaths",
            "--no-pager",
        ])
        .output()
        .map_err(|error| format!("无法查询 systemd 中的 Mihomo 服务：{error}"))?;
    let output = checked_output("systemctl show mihomo.service", output)?;
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "systemd 返回的 Mihomo 服务信息不是 UTF-8".to_string())?;
    let state = parse_systemd_unit(&output)?;
    validate_systemd_unit_state(&state, expectation)?;
    if let Some(path) = state.fragment_path.as_deref()
        && !trusted_root_file(path)
    {
        return Err(format!(
            "systemd 加载的 Mihomo 服务文件 {} 不是安全的 root 普通文件",
            path.display()
        ));
    }
    for path in &state.drop_in_paths {
        if !trusted_root_file(path) {
            return Err(format!(
                "systemd 加载的 Mihomo drop-in {} 不是安全的 root 普通文件",
                path.display()
            ));
        }
    }
    Ok(())
}

fn ensure_systemd() -> Result<(), String> {
    systemctl_path()?;
    if !Path::new("/run/systemd/system").is_dir() {
        return Err("当前系统没有可用的 systemd；请使用 --controller 连接外部 Mihomo".into());
    }
    Ok(())
}

fn install_for_apply() -> Result<(), String> {
    ensure_systemd()?;
    core_package::ensure_debian_host()?;
    validate_systemd_unit(UnitExpectation::Absent)?;
    let release = CoreRelease::embedded()?;
    let package = release.package_for(std::env::consts::OS, std::env::consts::ARCH)?;
    eprintln!(
        "未检测到 Mihomo，正在下载官方 {} 安装包...",
        release.recommended()
    );
    let artifact = core_package::download_package(&release, package)?;
    eprintln!("安装包校验通过，正在安装 Mihomo 服务...");
    core_package::install_deb(artifact.path())?;
    verify_installed_version()?;
    daemon_reload()?;
    eprintln!(
        "Mihomo {} 已安装，正在加载唯一配置。",
        release.recommended()
    );
    Ok(())
}

fn verify_installed_version() -> Result<(), String> {
    let binary = Path::new("/usr/bin/mihomo");
    if !trusted_root_file(binary) {
        return Err("安装完成后未找到受信任的 /usr/bin/mihomo".into());
    }
    validate_installed_core_at(binary, VersionRequirement::Recommended).map(|_| ())
}

fn daemon_reload() -> Result<(), String> {
    let systemctl = systemctl_path()?;
    let output = run_privileged(&systemctl, &["daemon-reload"])?;
    checked_output("systemctl daemon-reload", output).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_runtime_accepts_only_tested_core_versions() {
        for version in ["v1.19.28", "v1.19.29"] {
            let output = format!("Mihomo Meta {version} linux amd64 with go1.26.5");
            assert!(
                validate_installed_version_output(&output, VersionRequirement::Supported).is_ok()
            );
        }

        for version in ["v1.19.27", "v1.20.0"] {
            let output = format!("Mihomo Meta {version} linux amd64 with go1.26.5");
            assert!(
                validate_installed_version_output(&output, VersionRequirement::Supported).is_err()
            );
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

        let absent =
            parse_systemd_unit("LoadState=not-found\nFragmentPath=\nDropInPaths=\n").unwrap();
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
        crate::core_manager::install_version(&paths, &source, version).unwrap();
        crate::core_manager::switch_current(&paths, version).unwrap();

        let selected = mihomo_binary_path_at(&paths, &[]).unwrap();

        assert_eq!(selected, Some(paths.binary(version)));
        fs::remove_dir_all(root).unwrap();
    }
}
