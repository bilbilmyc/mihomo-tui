use super::{Compatibility, CoreRelease, CoreVersion};

const VALID_MANIFEST: &str = r#"{
    "schema": 1,
    "repository": "MetaCubeX/mihomo",
    "recommended": "v1.19.29",
    "minimum_supported": "v1.19.28",
    "maximum_exclusive": "v1.20.0",
    "license": {
        "spdx": "GPL-3.0",
        "asset": "LICENSE",
        "sha256": "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986"
    },
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
    assert_eq!(release.minimum_supported().to_string(), "v1.19.28");
    assert_eq!(release.maximum_exclusive().to_string(), "v1.20.0");
    assert_eq!(
        release.package_for("linux", "x86_64").unwrap().deb_arch,
        "amd64"
    );
    assert_eq!(
        release.package_for("linux", "aarch64").unwrap().deb_arch,
        "arm64"
    );
    assert_eq!(release.license().spdx, "GPL-3.0");
    assert_eq!(
        release.license().sha256,
        "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986"
    );
}

#[test]
fn manifest_validation_rejects_untrusted_or_ambiguous_metadata() {
    for invalid in [
        VALID_MANIFEST.replace("MetaCubeX/mihomo", "attacker/mihomo"),
        VALID_MANIFEST.replace("\"GPL-3.0\"", "\"MIT\""),
        VALID_MANIFEST.replace(
            "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591",
            "not-a-sha256",
        ),
        VALID_MANIFEST.replace("v1.20.0", "v1.19.28"),
        VALID_MANIFEST.replace(
            "    ]",
            "        ,{\"os\":\"linux\",\"arch\":\"x86_64\",\"asset\":\"duplicate.deb\",\"sha256\":\"6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591\",\"deb_arch\":\"amd64\",\"deb_version\":\"1.19.29\"}\n    ]",
        ),
    ] {
        assert!(CoreRelease::parse(&invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn mihomo_version_output_is_parsed_without_substring_matching() {
    assert_eq!(
        CoreVersion::from_mihomo_output("Mihomo Meta v1.19.28 linux amd64 with go1.26.5 Wed Jul 8")
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
