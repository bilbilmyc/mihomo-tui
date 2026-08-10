use serde::Deserialize;
use std::{collections::HashSet, fmt};

const OFFICIAL_REPOSITORY: &str = "MetaCubeX/mihomo";
const MANIFEST_SCHEMA: u32 = 1;
const EMBEDDED_MANIFEST: &str = include_str!("../managed-core.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoreVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl CoreVersion {
    pub fn parse(value: &str) -> Result<Self, String> {
        let numbers = value
            .strip_prefix('v')
            .ok_or_else(|| format!("Mihomo version must start with v: {value}"))?;
        let parts = numbers.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(format!(
                "Mihomo version must use vMAJOR.MINOR.PATCH: {value}"
            ));
        }
        let version = Self {
            major: parts[0]
                .parse()
                .map_err(|_| format!("invalid Mihomo major version: {value}"))?,
            minor: parts[1]
                .parse()
                .map_err(|_| format!("invalid Mihomo minor version: {value}"))?,
            patch: parts[2]
                .parse()
                .map_err(|_| format!("invalid Mihomo patch version: {value}"))?,
        };
        if version.to_string() != value {
            return Err(format!("Mihomo version is not canonical: {value}"));
        }
        Ok(version)
    }

    pub fn from_mihomo_output(output: &str) -> Result<Self, String> {
        let candidates = output
            .split_whitespace()
            .filter(|word| {
                word.as_bytes().first() == Some(&b'v')
                    && word.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            })
            .map(Self::parse)
            .collect::<Result<Vec<_>, _>>()?;
        match candidates.as_slice() {
            [version] => Ok(*version),
            [] => Err(format!(
                "Mihomo version output does not contain vMAJOR.MINOR.PATCH: {}",
                output.trim()
            )),
            _ => Err("Mihomo version output contains multiple version tags".into()),
        }
    }

    fn deb_version(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl fmt::Display for CoreVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Supported,
    TooOld,
    UntestedNewer,
}

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
    packages: Vec<CorePackage>,
}

#[derive(Debug, Clone)]
pub struct CoreRelease {
    recommended: CoreVersion,
    minimum_supported: CoreVersion,
    maximum_exclusive: CoreVersion,
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
    if package.sha256.len() != 64
        || !package
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"{
        "schema": 1,
        "repository": "MetaCubeX/mihomo",
        "recommended": "v1.19.29",
        "minimum_supported": "v1.19.28",
        "maximum_exclusive": "v1.20.0",
        "packages": [
            {
                "os": "linux",
                "arch": "x86_64",
                "asset": "mihomo-linux-amd64-v1-v1.19.29.deb",
                "sha256": "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591",
                "deb_arch": "amd64",
                "deb_version": "1.19.29"
            }
        ]
    }"#;

    #[test]
    fn embedded_manifest_defines_the_tested_release_contract() {
        let release = CoreRelease::embedded().unwrap();

        assert_eq!(release.recommended().to_string(), "v1.19.29");
        assert_eq!(release.minimum_supported.to_string(), "v1.19.28");
        assert_eq!(release.maximum_exclusive.to_string(), "v1.20.0");
        assert_eq!(
            release.package_for("linux", "x86_64").unwrap().deb_arch,
            "amd64"
        );
        assert_eq!(
            release.package_for("linux", "aarch64").unwrap().deb_arch,
            "arm64"
        );
    }

    #[test]
    fn manifest_validation_rejects_untrusted_or_ambiguous_metadata() {
        for invalid in [
            VALID_MANIFEST.replace("MetaCubeX/mihomo", "attacker/mihomo"),
            VALID_MANIFEST.replace(
                "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591",
                "not-a-sha256",
            ),
            VALID_MANIFEST.replace("v1.20.0", "v1.19.28"),
            VALID_MANIFEST.replace(
                "        ]",
                "            ,{\"os\":\"linux\",\"arch\":\"x86_64\",\"asset\":\"duplicate.deb\",\"sha256\":\"6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591\",\"deb_arch\":\"amd64\",\"deb_version\":\"1.19.29\"}\n        ]",
            ),
        ] {
            assert!(CoreRelease::parse(&invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn mihomo_version_output_is_parsed_without_substring_matching() {
        assert_eq!(
            CoreVersion::from_mihomo_output(
                "Mihomo Meta v1.19.28 linux amd64 with go1.26.5 Wed Jul 8"
            )
            .unwrap()
            .to_string(),
            "v1.19.28"
        );
        assert!(CoreVersion::from_mihomo_output("Mihomo Meta v1.19 linux").is_err());
        assert!(CoreVersion::from_mihomo_output("Mihomo Meta 1.19.28 linux").is_err());
        assert!(CoreVersion::from_mihomo_output("error mentions v1.19.28x").is_err());
    }

    #[test]
    fn compatibility_range_is_minimum_inclusive_and_maximum_exclusive() {
        let release = CoreRelease::parse(VALID_MANIFEST).unwrap();

        assert_eq!(
            release.compatibility(CoreVersion::parse("v1.19.27").unwrap()),
            Compatibility::TooOld
        );
        assert_eq!(
            release.compatibility(CoreVersion::parse("v1.19.28").unwrap()),
            Compatibility::Supported
        );
        assert_eq!(
            release.compatibility(CoreVersion::parse("v1.19.99").unwrap()),
            Compatibility::Supported
        );
        assert_eq!(
            release.compatibility(CoreVersion::parse("v1.20.0").unwrap()),
            Compatibility::UntestedNewer
        );
    }

    #[test]
    fn package_selection_only_accepts_declared_targets() {
        let release = CoreRelease::parse(VALID_MANIFEST).unwrap();

        assert_eq!(
            release.package_for("linux", "x86_64").unwrap().asset,
            "mihomo-linux-amd64-v1-v1.19.29.deb"
        );
        assert!(release.package_for("linux", "mips").is_err());
        assert!(release.package_for("macos", "x86_64").is_err());
    }
}
