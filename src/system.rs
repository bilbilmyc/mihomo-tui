use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    process::{Command, Output},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

#[cfg(unix)]
const ROOT_UID: u32 = 0;

pub struct RuntimeLock {
    _file: File,
}

pub fn acquire_runtime_lock() -> Result<RuntimeLock, String> {
    let path = Path::new("/run/mihomo-tui.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .map_err(|error| format!("无法打开 Mihomo 运行时锁：{error}"))?;
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .map_err(|error| format!("无法检查 Mihomo 运行时锁：{error}"))?;
        if !metadata.file_type().is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err("/run/mihomo-tui.lock 的所有者或权限不安全".into());
        }
    }
    file.try_lock()
        .map_err(|error| format!("另一个 mihomo-tui 正在管理本机服务：{error}"))?;
    Ok(RuntimeLock { _file: file })
}

#[cfg(unix)]
pub fn trusted_root_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_file()
        && owner_is_trusted(metadata.uid())
        && metadata.mode() & 0o022 == 0
}

#[cfg(not(unix))]
pub fn trusted_root_file(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
pub fn trusted_root_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_dir()
        && owner_is_trusted(metadata.uid())
        && metadata.mode() & 0o022 == 0
}

#[cfg(not(unix))]
pub fn trusted_root_directory(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
fn production_owner_is_trusted(uid: u32) -> bool {
    uid == ROOT_UID
}

#[cfg(all(unix, not(test)))]
fn owner_is_trusted(uid: u32) -> bool {
    production_owner_is_trusted(uid)
}

#[cfg(all(unix, test))]
fn owner_is_trusted(uid: u32) -> bool {
    production_owner_is_trusted(uid)
        || std::env::current_exe()
            .ok()
            .and_then(|path| fs::metadata(path).ok())
            .is_some_and(|metadata| metadata.uid() == uid)
}

#[cfg(unix)]
pub fn effective_root() -> bool {
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid() == 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn effective_root() -> bool {
    false
}

pub fn clean_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LANG", "C");
    command
}

pub fn run_privileged(program: &Path, args: &[&str]) -> Result<Output, String> {
    if !effective_root() {
        return Err("该操作需要 root 权限".into());
    }
    clean_command(program)
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 {}：{error}", program.display()))
}

pub fn checked_output(action: &str, output: Output) -> Result<Output, String> {
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "无诊断输出"
    };
    Err(format!("{action} 失败（{}）：{detail}", output.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn production_owner_policy_only_accepts_root() {
        assert!(production_owner_is_trusted(0));
        assert!(!production_owner_is_trusted(1));
        assert!(!production_owner_is_trusted(u32::MAX));
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_runtime_files_are_not_trusted() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Path::new("/tmp").join(format!(
            "mihomo-tui-untrusted-{}-{unique}",
            std::process::id()
        ));
        fs::write(&path, b"not executable").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();

        assert!(!trusted_root_file(&path));
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn root_owned_directories_must_not_be_group_or_world_writable() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Path::new("/tmp").join(format!(
            "mihomo-tui-directory-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(trusted_root_directory(&path));

        fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!trusted_root_directory(&path));
        fs::remove_dir(path).unwrap();
    }
}
