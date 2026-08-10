use super::{
    deb::{validate_deb_metadata, validate_extracted_candidate},
    download::{allowed_release_url, copy_verified},
};
use crate::core::CoreRelease;
use std::{fs, path::Path};

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
