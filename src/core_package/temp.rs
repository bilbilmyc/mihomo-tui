use crate::system::trusted_root_directory;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt};

pub(super) struct PrivateDirectory {
    pub(super) path: PathBuf,
}

impl PrivateDirectory {
    #[cfg(unix)]
    pub(super) fn create(prefix: &str) -> Result<Self, String> {
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
    pub(super) fn create(_prefix: &str) -> Result<Self, String> {
        Err("内核解包仅支持 Unix 系统".into())
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
pub(super) fn secure_temp_dir() -> Result<&'static Path, String> {
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
pub(super) fn secure_temp_dir() -> Result<&'static Path, String> {
    Err("自动安装仅支持 Unix 系统".into())
}

pub(super) fn random_hex(bytes: usize) -> Result<String, String> {
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
