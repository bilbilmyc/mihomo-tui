use super::{DownloadedPackage, deb::validate_deb_file};
use crate::core::{CorePackage, CoreRelease};
use sha2::{Digest, Sha256};
use std::{io::Read, io::Write, time::Duration};

const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn download_package(
    release: &CoreRelease,
    package: &CorePackage,
) -> Result<DownloadedPackage, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("Mihomo 安装包重定向次数过多")
        } else if allowed_release_url(attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("Mihomo 安装包被重定向到非 GitHub 域名")
        }
    });
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .redirect(policy)
        .user_agent(concat!("mihomo-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("无法创建下载客户端：{error}"))?;
    let mut response = client
        .get(release.package_url(package))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("下载 Mihomo 安装包失败：{error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PACKAGE_BYTES)
    {
        return Err(format!(
            "Mihomo 安装包过大（上限 {MAX_PACKAGE_BYTES} 字节）"
        ));
    }
    let (artifact, mut file) = DownloadedPackage::create()?;
    copy_verified(&mut response, &mut file, MAX_PACKAGE_BYTES, &package.sha256)?;
    file.sync_all()
        .map_err(|error| format!("无法同步临时安装包：{error}"))?;
    drop(file);
    validate_deb_file(artifact.path(), package)?;
    Ok(artifact)
}

pub(super) fn allowed_release_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host_str() {
        Some("github.com") => url
            .path()
            .starts_with("/MetaCubeX/mihomo/releases/download/"),
        Some(
            "release-assets.githubusercontent.com"
            | "objects.githubusercontent.com"
            | "github-releases.githubusercontent.com",
        ) => true,
        _ => false,
    }
}

pub(super) fn copy_verified(
    mut reader: impl Read,
    mut writer: impl Write,
    max_bytes: u64,
    expected_sha256: &str,
) -> Result<u64, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "Mihomo 安装包大小溢出".to_string())?;
        if total > max_bytes {
            return Err(format!("Mihomo 安装包过大（上限 {max_bytes} 字节）"));
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        return Err(format!(
            "Mihomo 安装包 SHA-256 校验失败：期望 {expected_sha256}，实际 {actual}"
        ));
    }
    Ok(total)
}
