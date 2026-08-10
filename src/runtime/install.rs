use super::{Inventory, systemd, systemd::UNIT_PATHS, systemd::UnitExpectation};
use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::{CorePaths, managed_active_version},
    core_package,
    system::{checked_output, clean_command, trusted_root_file},
};
use std::{fs, path::Path, path::PathBuf, process::Output};

const BINARY_PATHS: [&str; 2] = ["/usr/bin/mihomo", "/usr/local/bin/mihomo"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VersionRequirement {
    Supported,
    Recommended,
}

pub(super) fn prepare_existing_for_apply() -> Result<(), String> {
    validate_existing_install()?;
    validate_installed_core(VersionRequirement::Supported)?;
    systemd::validate_systemd_unit(UnitExpectation::Packaged)
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

pub(super) fn validate_installed_version_output(
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

pub(super) fn inspect() -> Result<Inventory, String> {
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

pub(super) fn mihomo_binary_path() -> Result<Option<PathBuf>, String> {
    mihomo_binary_path_at(
        &CorePaths::system(),
        &BINARY_PATHS.iter().map(PathBuf::from).collect::<Vec<_>>(),
    )
}

pub(super) fn mihomo_binary_path_at(
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

pub(super) fn install_for_apply() -> Result<(), String> {
    systemd::ensure_systemd()?;
    core_package::ensure_debian_host()?;
    systemd::validate_systemd_unit(UnitExpectation::Absent)?;
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
    systemd::daemon_reload()?;
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
