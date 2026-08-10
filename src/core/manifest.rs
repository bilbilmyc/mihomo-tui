use super::{Compatibility, CoreLicense, CoreVersion};
use serde::Deserialize;
use std::collections::HashSet;

const OFFICIAL_REPOSITORY: &str = "MetaCubeX/mihomo";
const MANIFEST_SCHEMA: u32 = 1;
const EMBEDDED_MANIFEST: &str = include_str!("../../managed-core.json");

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CorePackage {
    pub os: String,
    pub arch: String,
    pub asset: String,
    pub sha256: String,
    pub deb_arch: String,
    pub deb_version: String,
}

#[derive(Debug, Deserialize)]
struct ManifestDocument {
    schema: u32,
    repository: String,
    recommended: String,
    minimum_supported: String,
    maximum_exclusive: String,
    license: CoreLicense,
    packages: Vec<CorePackage>,
}

#[derive(Debug, Clone)]
pub struct CoreRelease {
    recommended: CoreVersion,
    minimum_supported: CoreVersion,
    maximum_exclusive: CoreVersion,
    license: CoreLicense,
    packages: Vec<CorePackage>,
}

impl CoreRelease {
    pub fn embedded() -> Result<Self, String> {
        Self::parse(EMBEDDED_MANIFEST)
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let document: ManifestDocument = serde_json::from_str(input)
            .map_err(|error| format!("invalid managed-core manifest: {error}"))?;
        if document.schema != MANIFEST_SCHEMA {
            return Err(format!(
                "unsupported managed-core manifest schema {}",
                document.schema
            ));
        }
        if document.repository != OFFICIAL_REPOSITORY {
            return Err(format!(
                "managed-core repository must be {OFFICIAL_REPOSITORY}"
            ));
        }
        let recommended = CoreVersion::parse(&document.recommended)?;
        let minimum_supported = CoreVersion::parse(&document.minimum_supported)?;
        let maximum_exclusive = CoreVersion::parse(&document.maximum_exclusive)?;
        if minimum_supported > recommended || recommended >= maximum_exclusive {
            return Err(
                "managed-core compatibility range does not contain the recommended version".into(),
            );
        }
        if document.packages.is_empty() {
            return Err("managed-core manifest must contain at least one package".into());
        }
        validate_license(&document.license)?;

        let mut targets = HashSet::new();
        for package in &document.packages {
            if !targets.insert((package.os.as_str(), package.arch.as_str())) {
                return Err(format!(
                    "managed-core manifest contains duplicate target {}/{}",
                    package.os, package.arch
                ));
            }
            validate_package(package, recommended)?;
        }

        Ok(Self {
            recommended,
            minimum_supported,
            maximum_exclusive,
            license: document.license,
            packages: document.packages,
        })
    }

    pub fn recommended(&self) -> CoreVersion {
        self.recommended
    }

    pub fn minimum_supported(&self) -> CoreVersion {
        self.minimum_supported
    }

    pub fn maximum_exclusive(&self) -> CoreVersion {
        self.maximum_exclusive
    }

    pub fn license(&self) -> &CoreLicense {
        &self.license
    }

    pub fn compatibility(&self, version: CoreVersion) -> Compatibility {
        if version < self.minimum_supported {
            Compatibility::TooOld
        } else if version >= self.maximum_exclusive {
            Compatibility::UntestedNewer
        } else {
            Compatibility::Supported
        }
    }

    pub fn require_supported(&self, version: CoreVersion) -> Result<(), String> {
        match self.compatibility(version) {
            Compatibility::Supported => Ok(()),
            Compatibility::TooOld => Err(format!(
                "已安装的 Mihomo {version} 低于受测最低版本 {}；请显式升级内核或使用 --controller",
                self.minimum_supported
            )),
            Compatibility::UntestedNewer => Err(format!(
                "已安装的 Mihomo {version} 尚未经过此版本 mihomo-tui 验证（受测范围为 {} 至 {}，不含上限）；请更新 mihomo-tui 或使用 --controller",
                self.minimum_supported, self.maximum_exclusive
            )),
        }
    }

    pub fn package_for(&self, os: &str, arch: &str) -> Result<&CorePackage, String> {
        self.packages
            .iter()
            .find(|package| package.os == os && package.arch == arch)
            .ok_or_else(|| format!("不支持为 {os}/{arch} 自动选择 Mihomo 安装包"))
    }

    pub fn package_url(&self, package: &CorePackage) -> String {
        format!(
            "https://github.com/{OFFICIAL_REPOSITORY}/releases/download/{}/{}",
            self.recommended, package.asset
        )
    }
}

fn validate_package(package: &CorePackage, recommended: CoreVersion) -> Result<(), String> {
    let safe_identifier = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    if !safe_identifier(&package.os)
        || !safe_identifier(&package.arch)
        || !safe_identifier(&package.asset)
        || !safe_identifier(&package.deb_arch)
        || !package.asset.ends_with(".deb")
    {
        return Err(format!(
            "managed-core package {}/{} contains an unsafe field",
            package.os, package.arch
        ));
    }
    if !valid_sha256(&package.sha256) {
        return Err(format!(
            "managed-core package {}/{} has an invalid SHA-256",
            package.os, package.arch
        ));
    }
    if package.deb_version != recommended.deb_version() {
        return Err(format!(
            "managed-core package {}/{} deb version does not match {}",
            package.os, package.arch, recommended
        ));
    }
    Ok(())
}

fn validate_license(license: &CoreLicense) -> Result<(), String> {
    if license.spdx != "GPL-3.0" || license.asset != "LICENSE" || !valid_sha256(&license.sha256) {
        return Err("managed-core license metadata is invalid".into());
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
