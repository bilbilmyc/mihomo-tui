use crate::{
    core::{CorePackage, CoreRelease},
    system::{
        checked_output, clean_command, run_privileged, trusted_root_directory, trusted_root_file,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;

pub struct DownloadedPackage {
    path: PathBuf,
}

impl DownloadedPackage {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn create() -> Result<(Self, File), String> {
        let name = format!("mihomo-tui-{}.deb", random_hex(16)?);
        let path = secure_temp_dir()?.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("无法创建临时文件 {}：{error}", path.display()))?;
        Ok((Self { path }, file))
    }
}

impl Drop for DownloadedPackage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct PrivateDirectory {
    path: PathBuf,
}

impl PrivateDirectory {
    #[cfg(unix)]
    fn create(prefix: &str) -> Result<Self, String> {
        let path = secure_temp_dir()?.join(format!("{prefix}-{}", random_hex(16)?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| format!("无法创建私有临时目录 {}：{error}", path.display()))?;
        if !trusted_root_directory(&path) {
            let _ = fs::remove_dir(&path);
            return Err(format!("私有临时目录 {} 的权限不安全", path.display()));
        }
        Ok(Self { path })
    }

    #[cfg(not(unix))]
    fn create(_prefix: &str) -> Result<Self, String> {
        Err("内核解包仅支持 Unix 系统".into())
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub struct ExtractedCore {
    _directory: PrivateDirectory,
    candidate: PathBuf,
}

impl ExtractedCore {
    pub fn candidate(&self) -> &Path {
        &self.candidate
    }
}

pub fn download_package(
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

pub fn ensure_debian_host() -> Result<(), String> {
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

pub fn install_deb(path: &Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "临时安装包路径不是 UTF-8".to_string())?;
    let output = run_privileged(
        Path::new("/usr/bin/dpkg"),
        &["--force-confold", "--install", path],
    )?;
    checked_output("dpkg 安装 Mihomo", output).map(|_| ())
}

pub fn extract_core(package: &DownloadedPackage) -> Result<ExtractedCore, String> {
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

fn validate_extracted_candidate(root: &Path) -> Result<PathBuf, String> {
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

fn validate_deb_file(path: &Path, package: &CorePackage) -> Result<(), String> {
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

fn validate_deb_metadata(metadata: &str, package: &CorePackage) -> Result<(), String> {
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

fn allowed_release_url(url: &reqwest::Url) -> bool {
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

#[cfg(unix)]
fn secure_temp_dir() -> Result<&'static Path, String> {
    let path = Path::new("/tmp");
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法检查系统临时目录 /tmp：{error}"))?;
    let mode = metadata.mode();
    if !metadata.file_type().is_dir()
        || metadata.uid() != 0
        || (mode & 0o022 != 0 && mode & 0o1000 == 0)
    {
        return Err("/tmp 的所有者或权限不安全，拒绝创建特权临时文件".into());
    }
    Ok(path)
}

#[cfg(not(unix))]
fn secure_temp_dir() -> Result<&'static Path, String> {
    Err("自动安装仅支持 Unix 系统".into())
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut random = vec![0_u8; bytes];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| format!("无法从系统随机源读取数据：{error}"))?;
    let mut encoded = String::with_capacity(bytes * 2);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(encoded)
}

fn copy_verified(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn supported_debian_architectures_use_pinned_packages() {
        let release = CoreRelease::embedded().unwrap();
        let amd64 = release.package_for("linux", "x86_64").unwrap();
        assert_eq!(amd64.asset, "mihomo-linux-amd64-v1-v1.19.29.deb");
        assert_eq!(
            amd64.sha256,
            "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591"
        );

        let arm64 = release.package_for("linux", "aarch64").unwrap();
        assert_eq!(arm64.asset, "mihomo-linux-arm64-v1.19.29.deb");
        assert_eq!(
            arm64.sha256,
            "a14e694a2bac6ca3848e05f4ef27596c5982dab812c23743823e7e5c35f7cfc9"
        );
        assert!(release.package_for("linux", "mips").is_err());
        assert!(release.package_for("macos", "x86_64").is_err());
    }

    #[test]
    fn package_bytes_are_limited_and_sha256_verified() {
        let mut output = Vec::new();
        copy_verified(
            std::io::Cursor::new(b"abc"),
            &mut output,
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        assert_eq!(output, b"abc");

        let too_large = copy_verified(
            std::io::Cursor::new(b"abcd"),
            Vec::new(),
            3,
            "88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589",
        )
        .unwrap_err();
        assert!(too_large.contains("过大"));

        let mismatch = copy_verified(
            std::io::Cursor::new(b"abc"),
            Vec::new(),
            3,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(mismatch.contains("SHA-256"));
    }

    #[test]
    fn deb_metadata_must_match_the_pinned_package() {
        let release = CoreRelease::embedded().unwrap();
        let package = release.package_for("linux", "x86_64").unwrap();
        validate_deb_metadata(
            "Package: mihomo\nVersion: 1.19.29\nArchitecture: amd64\n",
            package,
        )
        .unwrap();

        assert!(
            validate_deb_metadata(
                "Package: other\nVersion: 1.19.29\nArchitecture: amd64\n",
                package,
            )
            .is_err()
        );
        assert!(
            validate_deb_metadata(
                "Package: mihomo\nVersion: 1.19.29\nArchitecture: arm64\n",
                package,
            )
            .is_err()
        );
    }

    #[test]
    fn download_url_and_redirects_are_restricted_to_github_release_hosts() {
        let release = CoreRelease::embedded().unwrap();
        let package = release.package_for("linux", "x86_64").unwrap();
        assert_eq!(
            release.package_url(package),
            "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.29/mihomo-linux-amd64-v1-v1.19.29.deb"
        );

        assert!(allowed_release_url(
            &reqwest::Url::parse("https://github.com/MetaCubeX/mihomo/releases/download/v/file")
                .unwrap()
        ));
        assert!(allowed_release_url(
            &reqwest::Url::parse("https://release-assets.githubusercontent.com/file").unwrap()
        ));
        assert!(!allowed_release_url(
            &reqwest::Url::parse("http://github.com/file").unwrap()
        ));
        assert!(!allowed_release_url(
            &reqwest::Url::parse("https://example.com/file").unwrap()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn extracted_candidate_must_be_the_exact_regular_executable() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Path::new("/tmp").join(format!(
            "mihomo-tui-extracted-{}-{unique}",
            std::process::id()
        ));
        let binary = root.join("usr/bin/mihomo");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"candidate").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(validate_extracted_candidate(&root).unwrap(), binary);

        fs::remove_file(&binary).unwrap();
        symlink("/usr/bin/mihomo", &binary).unwrap();
        assert!(validate_extracted_candidate(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
