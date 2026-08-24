use super::{install::mihomo_binary_path, root_required_message};
use crate::{
    core_manager::CorePaths,
    system::{checked_output, clean_command, effective_root, run_privileged, trusted_root_file},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

pub(super) const UNIT_PATHS: [&str; 3] = [
    "/etc/systemd/system/mihomo.service",
    "/usr/lib/systemd/system/mihomo.service",
    "/lib/systemd/system/mihomo.service",
];
pub(super) const MANAGED_DROP_IN: &str = "/etc/systemd/system/mihomo.service.d/10-mihomo-tui.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnitExpectation {
    Absent,
    Packaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SystemdUnitState {
    load_state: String,
    fragment_path: Option<PathBuf>,
    drop_in_paths: Vec<PathBuf>,
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

pub(super) fn config_apply_command() -> (&'static str, [&'static str; 2]) {
    (
        "systemctl restart mihomo.service",
        ["restart", "mihomo.service"],
    )
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

pub(super) fn core_upgrade_restart_command() -> (&'static str, [&'static str; 2]) {
    (
        "systemctl restart mihomo.service",
        ["restart", "mihomo.service"],
    )
}

pub(super) fn workspace_drop_in_content(binary: &Path) -> String {
    format!(
        "[Service]\nExecStart=\nExecStart={} -d /etc/mihomo-tui\n",
        binary.display()
    )
}

fn systemctl_path() -> Result<PathBuf, String> {
    ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| trusted_root_file(path))
        .ok_or_else(|| "找不到受信任的 systemctl 可执行文件".to_string())
}

pub(super) fn parse_systemd_unit(output: &str) -> Result<SystemdUnitState, String> {
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

pub(super) fn validate_systemd_unit_state(
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

pub(super) fn validate_systemd_unit(expectation: UnitExpectation) -> Result<(), String> {
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

pub(super) fn ensure_systemd() -> Result<(), String> {
    systemctl_path()?;
    if !Path::new("/run/systemd/system").is_dir() {
        return Err("当前系统没有可用的 systemd；请使用 --controller 连接外部 Mihomo".into());
    }
    Ok(())
}

pub(super) fn daemon_reload() -> Result<(), String> {
    let systemctl = systemctl_path()?;
    let output = run_privileged(&systemctl, &["daemon-reload"])?;
    checked_output("systemctl daemon-reload", output).map(|_| ())
}
