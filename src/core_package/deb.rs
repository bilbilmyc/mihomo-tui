use super::{DownloadedPackage, ExtractedCore, temp::PrivateDirectory};
use crate::{
    core::CorePackage,
    system::{
        checked_output, clean_command, run_privileged, trusted_root_directory, trusted_root_file,
    },
};
use std::{fs, path::Path, path::PathBuf};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(super) fn ensure_debian_host() -> Result<(), String> {
    if !Path::new("/etc/debian_version").is_file() {
        return Err("自动安装目前仅支持 Debian/Ubuntu；请使用 --controller 连接外部 Mihomo".into());
    }
    for tool in ["/usr/bin/dpkg", "/usr/bin/dpkg-deb", "/usr/bin/install"] {
        if !trusted_root_file(Path::new(tool)) {
            return Err(format!("找不到受信任的系统工具 {tool}"));
        }
    }
    Ok(())
}

pub(super) fn install_deb(path: &Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "临时安装包路径不是 UTF-8".to_string())?;
    let output = run_privileged(
        Path::new("/usr/bin/dpkg"),
        &["--force-confold", "--install", path],
    )?;
    checked_output("dpkg 安装 Mihomo", output).map(|_| ())
}

pub(super) fn extract_core(package: &DownloadedPackage) -> Result<ExtractedCore, String> {
    let dpkg_deb = Path::new("/usr/bin/dpkg-deb");
    if !trusted_root_file(dpkg_deb) {
        return Err("找不到受信任的 /usr/bin/dpkg-deb".into());
    }
    let directory = PrivateDirectory::create("mihomo-tui-core")?;
    let output = clean_command(dpkg_deb)
        .arg("--extract")
        .arg(package.path())
        .arg(&directory.path)
        .output()
        .map_err(|error| format!("无法解包 Mihomo deb：{error}"))?;
    checked_output("dpkg-deb --extract", output)?;
    let candidate = validate_extracted_candidate(&directory.path)?;
    Ok(ExtractedCore {
        _directory: directory,
        candidate,
    })
}

pub(super) fn validate_extracted_candidate(root: &Path) -> Result<PathBuf, String> {
    let usr = root.join("usr");
    let bin = usr.join("bin");
    for directory in [root, usr.as_path(), bin.as_path()] {
        if !trusted_root_directory(directory) {
            return Err(format!(
                "解包后的 Mihomo 目录 {} 的所有者或权限不安全",
                directory.display()
            ));
        }
    }
    let candidate = bin.join("mihomo");
    if !trusted_root_file(&candidate) {
        return Err(format!(
            "解包后未找到安全的普通文件 {}",
            candidate.display()
        ));
    }
    #[cfg(unix)]
    {
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|error| format!("无法检查 Mihomo 候选文件：{error}"))?;
        if metadata.mode() & 0o111 == 0 {
            return Err("解包后的 Mihomo 候选文件不可执行".into());
        }
    }
    Ok(candidate)
}

pub(super) fn validate_deb_file(path: &Path, package: &CorePackage) -> Result<(), String> {
    let dpkg_deb = Path::new("/usr/bin/dpkg-deb");
    if !trusted_root_file(dpkg_deb) {
        return Err("找不到受信任的 /usr/bin/dpkg-deb".into());
    }
    let output = clean_command(dpkg_deb)
        .arg("--field")
        .arg(path)
        .args(["Package", "Version", "Architecture"])
        .output()
        .map_err(|error| format!("无法检查 Mihomo deb 元数据：{error}"))?;
    let output = checked_output("dpkg-deb --field", output)?;
    let metadata =
        String::from_utf8(output.stdout).map_err(|_| "Mihomo deb 元数据不是 UTF-8".to_string())?;
    validate_deb_metadata(&metadata, package)
}

pub(super) fn validate_deb_metadata(metadata: &str, package: &CorePackage) -> Result<(), String> {
    let field = |name: &str| {
        metadata.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim())
        })
    };
    let expected = [
        ("Package", "mihomo"),
        ("Version", package.deb_version.as_str()),
        ("Architecture", package.deb_arch.as_str()),
    ];
    for (name, expected_value) in expected {
        let actual = field(name).ok_or_else(|| format!("安装包缺少 {name} 元数据"))?;
        if actual != expected_value {
            return Err(format!(
                "安装包 {name} 不匹配：期望 {expected_value}，实际 {actual}"
            ));
        }
    }
    Ok(())
}
