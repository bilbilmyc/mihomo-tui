mod deb;
mod download;
mod temp;

#[cfg(test)]
mod tests;

use crate::core::{CorePackage, CoreRelease};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use temp::{PrivateDirectory, random_hex, secure_temp_dir};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

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
    download::download_package(release, package)
}

pub fn ensure_debian_host() -> Result<(), String> {
    deb::ensure_debian_host()
}

pub fn install_deb(path: &Path) -> Result<(), String> {
    deb::install_deb(path)
}

pub fn extract_core(package: &DownloadedPackage) -> Result<ExtractedCore, String> {
    deb::extract_core(package)
}
