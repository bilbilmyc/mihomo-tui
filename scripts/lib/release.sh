#!/usr/bin/env bash

release_fail() {
  echo "$*" >&2
  return 1
}

release_require_tools() {
  local tool
  for tool in "$@"; do
    command -v "$tool" >/dev/null 2>&1 || release_fail "required tool is missing: $tool" || return 1
  done
}

# Target variables are the interface exported to the sourcing builders.
# shellcheck disable=SC2034
release_load_target() {
  local requested_arch=$1
  local package_json expected_asset

  case $requested_arch in
    amd64 | x86_64)
      RELEASE_RUST_ARCH=x86_64
      RELEASE_DEB_ARCH=amd64
      RELEASE_RPM_ARCH=x86_64
      ;;
    arm64 | aarch64)
      RELEASE_RUST_ARCH=aarch64
      RELEASE_DEB_ARCH=arm64
      RELEASE_RPM_ARCH=aarch64
      ;;
    *) release_fail "unsupported Linux architecture: $requested_arch" || return 1 ;;
  esac

  package_json=$(jq -ce --arg rust_arch "$RELEASE_RUST_ARCH" '
    [.packages[] | select(.os == "linux" and .arch == $rust_arch)]
    | if length == 1 then .[0] else error("manifest target is missing or duplicated") end
  ' "$RELEASE_MANIFEST")
  RELEASE_CORE_TAG=$(jq -er '.recommended' "$RELEASE_MANIFEST")
  RELEASE_CORE_ASSET=$(jq -er '.asset' <<<"$package_json")
  RELEASE_CORE_SHA256=$(jq -er '.sha256' <<<"$package_json")
  RELEASE_CORE_DEB_VERSION=$(jq -er '.deb_version' <<<"$package_json")
  RELEASE_MANIFEST_DEB_ARCH=$(jq -er '.deb_arch' <<<"$package_json")
  RELEASE_LICENSE_ASSET=$(jq -er '.license.asset' "$RELEASE_MANIFEST")
  RELEASE_LICENSE_SHA256=$(jq -er '.license.sha256' "$RELEASE_MANIFEST")
  RELEASE_LICENSE_SPDX=$(jq -er '.license.spdx' "$RELEASE_MANIFEST")

  [[ $RELEASE_CORE_TAG =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
    release_fail "managed core tag is invalid: $RELEASE_CORE_TAG" || return 1
  case $RELEASE_RUST_ARCH in
    x86_64) expected_asset="mihomo-linux-amd64-v1-$RELEASE_CORE_TAG.deb" ;;
    aarch64) expected_asset="mihomo-linux-arm64-$RELEASE_CORE_TAG.deb" ;;
  esac
  [[ $RELEASE_CORE_ASSET == "$expected_asset" ]] ||
    release_fail "manifest asset does not match $RELEASE_RUST_ARCH naming policy: $RELEASE_CORE_ASSET" || return 1
  [[ $RELEASE_MANIFEST_DEB_ARCH == "$RELEASE_DEB_ARCH" ]] ||
    release_fail "manifest Debian architecture mismatch" || return 1
  [[ $RELEASE_CORE_DEB_VERSION == "${RELEASE_CORE_TAG#v}" ]] ||
    release_fail "manifest core version mismatch" || return 1
  [[ $RELEASE_CORE_SHA256 =~ ^[0-9a-f]{64}$ ]] ||
    release_fail "manifest core SHA-256 is invalid" || return 1
  [[ $RELEASE_LICENSE_ASSET == LICENSE && $RELEASE_LICENSE_SPDX == GPL-3.0 ]] ||
    release_fail "manifest license identity is invalid" || return 1
  [[ $RELEASE_LICENSE_SHA256 =~ ^[0-9a-f]{64}$ ]] ||
    release_fail "manifest license SHA-256 is invalid" || return 1
}

release_require_native_arch() {
  local native_arch
  native_arch=$(uname -m)
  [[ $native_arch == "$RELEASE_RUST_ARCH" ]] ||
    release_fail "release builds must run natively on $RELEASE_RUST_ARCH (host is $native_arch)"
}

release_read_app_version() {
  RELEASE_APP_VERSION=$(cargo metadata \
    --manifest-path "$RELEASE_REPO_ROOT/Cargo.toml" \
    --no-deps \
    --format-version 1 |
    jq -er '.packages[] | select(.name == "mihomo-tui") | .version')
}

release_prepare_binary() {
  local binary=$1
  local binary_explicit=$2
  local description

  if [[ $binary_explicit == false ]]; then
    cargo build --manifest-path "$RELEASE_REPO_ROOT/Cargo.toml" --release --locked
  fi
  [[ -f $binary && -x $binary && ! -L $binary ]] ||
    release_fail "mihomo-tui binary is not a regular executable: $binary" || return 1
  RELEASE_BINARY=$(realpath "$binary")
  description=$(file -b "$RELEASE_BINARY")
  [[ $description == *ELF* ]] || release_fail "mihomo-tui binary is not ELF: $description" || return 1
  case $RELEASE_RUST_ARCH in
    x86_64) [[ $description == *"x86-64"* ]] ;;
    aarch64) [[ $description == *"aarch64"* || $description == *"ARM64"* ]] ;;
  esac || release_fail "mihomo-tui binary architecture mismatch: $description" || return 1
  [[ $("$RELEASE_BINARY" --version) == "mihomo-tui $RELEASE_APP_VERSION" ]] ||
    release_fail "mihomo-tui binary version does not match $RELEASE_APP_VERSION"
}

release_prepare_core() {
  local core_deb=$1
  local work_dir=$2
  local actual_sha256 version_output tag_pattern

  if [[ -z $core_deb ]]; then
    core_deb="$work_dir/$RELEASE_CORE_ASSET"
    curl --fail --silent --show-error --location \
      --retry 3 --retry-all-errors \
      --max-filesize 134217728 \
      --proto '=https' --proto-redir '=https' \
      --user-agent "mihomo-tui-release-builder" \
      --output "$core_deb" \
      "https://github.com/MetaCubeX/mihomo/releases/download/$RELEASE_CORE_TAG/$RELEASE_CORE_ASSET"
  fi
  [[ -f $core_deb && ! -L $core_deb ]] ||
    release_fail "core deb is not a regular file: $core_deb" || return 1
  [[ $(stat -c '%s' "$core_deb") -le 134217728 ]] ||
    release_fail "core deb exceeds the 128 MiB limit" || return 1
  actual_sha256=$(sha256sum "$core_deb")
  actual_sha256=${actual_sha256%% *}
  [[ $actual_sha256 == "$RELEASE_CORE_SHA256" ]] || release_fail "core deb SHA-256 mismatch" || return 1
  [[ $(dpkg-deb --field "$core_deb" Package) == mihomo ]] || release_fail "core package name mismatch" || return 1
  [[ $(dpkg-deb --field "$core_deb" Version) == "$RELEASE_CORE_DEB_VERSION" ]] ||
    release_fail "core package version mismatch" || return 1
  [[ $(dpkg-deb --field "$core_deb" Architecture) == "$RELEASE_DEB_ARCH" ]] ||
    release_fail "core package architecture mismatch" || return 1

  RELEASE_CORE_ROOT="$work_dir/extracted-core"
  mkdir -m 700 "$RELEASE_CORE_ROOT"
  dpkg-deb --extract "$core_deb" "$RELEASE_CORE_ROOT"
  RELEASE_CORE_BINARY="$RELEASE_CORE_ROOT/usr/bin/mihomo"
  [[ -f $RELEASE_CORE_BINARY && -x $RELEASE_CORE_BINARY && ! -L $RELEASE_CORE_BINARY ]] ||
    release_fail "official deb does not contain a regular usr/bin/mihomo executable" || return 1
  version_output=$("$RELEASE_CORE_BINARY" -v)
  tag_pattern=${RELEASE_CORE_TAG//./\.}
  [[ $version_output =~ (^|[[:space:]])$tag_pattern([[:space:]]|$) ]] ||
    release_fail "official core version does not match $RELEASE_CORE_TAG: $version_output"
}

release_prepare_license() {
  local license_file=$1
  local work_dir=$2
  local actual_sha256

  if [[ -z $license_file ]]; then
    license_file="$work_dir/$RELEASE_LICENSE_ASSET"
    curl --fail --silent --show-error --location \
      --retry 3 --retry-all-errors \
      --max-filesize 1048576 \
      --proto '=https' --proto-redir '=https' \
      --user-agent "mihomo-tui-release-builder" \
      --output "$license_file" \
      "https://raw.githubusercontent.com/MetaCubeX/mihomo/$RELEASE_CORE_TAG/$RELEASE_LICENSE_ASSET"
  fi
  [[ -f $license_file && ! -L $license_file ]] ||
    release_fail "Mihomo license is not a regular file: $license_file" || return 1
  RELEASE_LICENSE_FILE=$(realpath "$license_file")
  [[ $(stat -c '%s' "$RELEASE_LICENSE_FILE") -le 1048576 ]] ||
    release_fail "Mihomo license exceeds the 1 MiB limit" || return 1
  actual_sha256=$(sha256sum "$RELEASE_LICENSE_FILE")
  actual_sha256=${actual_sha256%% *}
  [[ $actual_sha256 == "$RELEASE_LICENSE_SHA256" ]] || release_fail "Mihomo license SHA-256 mismatch"
}

release_write_notice() {
  local destination=$1
  {
    echo "Bundled component: Mihomo $RELEASE_CORE_TAG"
    echo "Source: https://github.com/MetaCubeX/mihomo/tree/$RELEASE_CORE_TAG"
    echo "License: $RELEASE_LICENSE_SPDX"
    echo "License file: /usr/share/doc/mihomo-tui/Mihomo-LICENSE"
    echo "Core package SHA-256: $RELEASE_CORE_SHA256"
  } >"$destination"
  chmod 644 "$destination"
}

release_write_checksum() {
  local artifact=$1
  (
    cd "$(dirname "$artifact")" || return 1
    sha256sum "$(basename "$artifact")" >"$(basename "$artifact").sha256"
    chmod 644 "$(basename "$artifact").sha256"
  )
}
