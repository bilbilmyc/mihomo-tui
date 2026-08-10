use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(unix)]
pub fn trusted_root_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0
}

#[cfg(not(unix))]
pub fn trusted_root_file(_path: &Path) -> bool {
    false
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
}
