#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$repo_root/managed-core.json"
architecture=$(dpkg --print-architecture)
core_deb=""
binary="$repo_root/target/release/mihomo-tui"
binary_explicit=false
output_dir="$repo_root/dist"

usage() {
  echo "usage: $0 [--architecture amd64|arm64] [--core-deb PATH] [--binary PATH] [--output-dir DIR]" >&2
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --architecture)
      architecture=${2:?missing architecture}
      shift 2
      ;;
    --core-deb)
      core_deb=${2:?missing core deb path}
      shift 2
      ;;
    --binary)
      binary=${2:?missing mihomo-tui binary path}
      binary_explicit=true
      shift 2
      ;;
    --output-dir)
      output_dir=${2:?missing output directory}
      shift 2
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

for tool in awk cargo chmod curl dpkg dpkg-deb dpkg-shlibdeps du file install jq ln mkdir mktemp realpath rm sed sha256sum stat; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done

case $architecture in
  amd64)
    rust_arch=x86_64
    core_asset_prefix=mihomo-linux-amd64-v1-
    ;;
  arm64)
    rust_arch=aarch64
    core_asset_prefix=mihomo-linux-arm64-
    ;;
  *)
    echo "unsupported Debian architecture: $architecture" >&2
    exit 1
    ;;
esac

native_arch=$(dpkg --print-architecture)
[[ $native_arch == "$architecture" ]] || {
  echo "package builds must run natively on $architecture (host is $native_arch)" >&2
  exit 1
}

package_json=$(jq -ce --arg rust_arch "$rust_arch" --arg deb_arch "$architecture" '
  [.packages[] | select(.os == "linux" and .arch == $rust_arch and .deb_arch == $deb_arch)]
  | if length == 1 then .[0] else error("manifest target is missing or duplicated") end
' "$manifest")
core_tag=$(jq -er '.recommended' "$manifest")
core_asset=$(jq -er '.asset' <<<"$package_json")
core_sha256=$(jq -er '.sha256' <<<"$package_json")
core_deb_version=$(jq -er '.deb_version' <<<"$package_json")
license_asset=$(jq -er '.license.asset' "$manifest")
license_sha256=$(jq -er '.license.sha256' "$manifest")
license_spdx=$(jq -er '.license.spdx' "$manifest")
[[ $core_tag =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
[[ $core_asset == "$core_asset_prefix$core_tag.deb" ]] || {
  echo "manifest asset does not match $architecture naming policy: $core_asset" >&2
  exit 1
}
[[ $core_deb_version == "${core_tag#v}" ]]
[[ $core_sha256 =~ ^[0-9a-f]{64}$ ]]
[[ $license_asset == LICENSE && $license_spdx == GPL-3.0 ]]
[[ $license_sha256 =~ ^[0-9a-f]{64}$ ]]

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT

if [[ -z $core_deb ]]; then
  core_deb="$temp_dir/$core_asset"
  core_url="https://github.com/MetaCubeX/mihomo/releases/download/$core_tag/$core_asset"
  curl --fail --silent --show-error --location \
    --retry 3 --retry-all-errors \
    --max-filesize 134217728 \
    --proto '=https' --proto-redir '=https' \
    --user-agent "mihomo-tui-deb-builder" \
    --output "$core_deb" "$core_url"
fi
[[ -f $core_deb && ! -L $core_deb ]] || {
  echo "core deb is not a regular file: $core_deb" >&2
  exit 1
}
[[ $(stat -c '%s' "$core_deb") -le 134217728 ]] || {
  echo "core deb exceeds the 128 MiB limit" >&2
  exit 1
}
actual_core_sha256=$(sha256sum "$core_deb")
actual_core_sha256=${actual_core_sha256%% *}
[[ $actual_core_sha256 == "$core_sha256" ]] || {
  echo "core deb SHA-256 mismatch" >&2
  exit 1
}
[[ $(dpkg-deb --field "$core_deb" Package) == mihomo ]]
[[ $(dpkg-deb --field "$core_deb" Version) == "$core_deb_version" ]]
[[ $(dpkg-deb --field "$core_deb" Architecture) == "$architecture" ]]

app_version=$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 | jq -er '.packages[] | select(.name == "mihomo-tui") | .version')
if [[ $binary_explicit == false ]]; then
  cargo build --manifest-path "$repo_root/Cargo.toml" --release --locked
fi
[[ -f $binary && -x $binary && ! -L $binary ]] || {
  echo "mihomo-tui binary is not a regular executable: $binary" >&2
  exit 1
}
binary=$(realpath "$binary")
binary_description=$(file -b "$binary")
case $architecture in
  amd64) [[ $binary_description == *"x86-64"* ]] ;;
  arm64) [[ $binary_description == *"aarch64"* || $binary_description == *"ARM64"* ]] ;;
esac || {
  echo "mihomo-tui binary architecture does not match $architecture: $binary_description" >&2
  exit 1
}
[[ $("$binary" --version) == "mihomo-tui $app_version" ]] || {
  echo "mihomo-tui binary version does not match package version $app_version" >&2
  exit 1
}

shlibs_workspace="$temp_dir/shlibs"
mkdir -p "$shlibs_workspace/debian/mihomo-tui/DEBIAN"
shlibs_binary="$shlibs_workspace/debian/mihomo-tui/usr/bin/mihomo-tui"
install -D -m 755 "$binary" "$shlibs_binary"
{
  echo "Source: mihomo-tui"
  echo "Section: net"
  echo "Priority: optional"
  echo "Maintainer: mihomo-tui maintainers"
  echo
  echo "Package: mihomo-tui"
  echo "Architecture: any"
  echo "Depends: \${shlibs:Depends}"
  echo "Description: temporary metadata for dependency calculation"
} >"$shlibs_workspace/debian/control"
shlibs_assignment=$(
  cd "$shlibs_workspace"
  dpkg-shlibdeps -O "debian/mihomo-tui/usr/bin/mihomo-tui"
)
shared_dependencies=${shlibs_assignment#shlibs:Depends=}
[[ -n $shared_dependencies && $shared_dependencies != "$shlibs_assignment" ]] || {
  echo "could not determine mihomo-tui shared library dependencies" >&2
  exit 1
}

license_file="$temp_dir/$license_asset"
license_url="https://raw.githubusercontent.com/MetaCubeX/mihomo/$core_tag/$license_asset"
curl --fail --silent --show-error --location \
  --retry 3 --retry-all-errors \
  --max-filesize 1048576 \
  --proto '=https' --proto-redir '=https' \
  --user-agent "mihomo-tui-deb-builder" \
  --output "$license_file" "$license_url"
[[ $(stat -c '%s' "$license_file") -le 1048576 ]] || {
  echo "Mihomo license exceeds the 1 MiB limit" >&2
  exit 1
}
actual_license_sha256=$(sha256sum "$license_file")
actual_license_sha256=${actual_license_sha256%% *}
[[ $actual_license_sha256 == "$license_sha256" ]] || {
  echo "Mihomo license SHA-256 mismatch" >&2
  exit 1
}

extracted_core="$temp_dir/extracted-core"
mkdir -m 700 "$extracted_core"
dpkg-deb --extract "$core_deb" "$extracted_core"
core_binary="$extracted_core/usr/bin/mihomo"
[[ -f $core_binary && -x $core_binary && ! -L $core_binary ]] || {
  echo "official deb does not contain a regular usr/bin/mihomo executable" >&2
  exit 1
}
core_version_output=$("$core_binary" -v)
core_tag_pattern=${core_tag//./\\.}
[[ $core_version_output =~ (^|[[:space:]])$core_tag_pattern([[:space:]]|$) ]] || {
  echo "official core version does not match $core_tag: $core_version_output" >&2
  exit 1
}

package_version="${app_version}+mihomo${core_deb_version}-1"
package_root="$temp_dir/package"
install -d -m 755 \
  "$package_root/DEBIAN" \
  "$package_root/etc/mihomo-tui" \
  "$package_root/usr/bin" \
  "$package_root/usr/lib/mihomo-tui/bundled/$core_tag" \
  "$package_root/usr/lib/systemd/system" \
  "$package_root/usr/share/doc/mihomo-tui"
install -m 755 "$binary" "$package_root/usr/bin/mihomo-tui"
install -m 755 "$core_binary" "$package_root/usr/lib/mihomo-tui/bundled/$core_tag/mihomo"
install -m 644 "$repo_root/packaging/debian/mihomo.service" "$package_root/usr/lib/systemd/system/mihomo.service"
install -m 644 "$license_file" "$package_root/usr/share/doc/mihomo-tui/Mihomo-LICENSE"

source_url="https://github.com/MetaCubeX/mihomo/tree/$core_tag"
{
  echo "Bundled component: Mihomo $core_tag"
  echo "Source: $source_url"
  echo "License: $license_spdx"
  echo "License file: /usr/share/doc/mihomo-tui/Mihomo-LICENSE"
  echo "Core package SHA-256: $core_sha256"
} >"$package_root/usr/share/doc/mihomo-tui/Mihomo-NOTICE"
chmod 644 "$package_root/usr/share/doc/mihomo-tui/Mihomo-NOTICE"
install -m 644 "$package_root/usr/share/doc/mihomo-tui/Mihomo-NOTICE" "$package_root/usr/share/doc/mihomo-tui/copyright"

sed "s|@CORE_TAG@|$core_tag|g" "$repo_root/packaging/debian/postinst" >"$package_root/DEBIAN/postinst"
chmod 755 "$package_root/DEBIAN/postinst"
install -m 755 "$repo_root/packaging/debian/preinst" "$package_root/DEBIAN/preinst"
install -m 755 "$repo_root/packaging/debian/prerm" "$package_root/DEBIAN/prerm"
install -m 755 "$repo_root/packaging/debian/postrm" "$package_root/DEBIAN/postrm"
installed_size=$(du -sk "$package_root/usr" "$package_root/etc" | awk '{ total += $1 } END { print total }')
{
  echo "Package: mihomo-tui"
  echo "Version: $package_version"
  echo "Architecture: $architecture"
  echo "Maintainer: mihomo-tui maintainers"
  echo "Section: net"
  echo "Priority: optional"
  echo "Installed-Size: $installed_size"
  echo "Depends: $shared_dependencies, ca-certificates, systemd"
  echo "Recommends: iproute2"
  echo "Conflicts: mihomo"
  echo "Description: SSH-friendly terminal control center with a pinned Mihomo core"
  echo " Ships mihomo-tui and the reviewed official Mihomo $core_tag core in a"
  echo " versioned layout. Installation does not automatically start the service."
} >"$package_root/DEBIAN/control"

mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
deb_name="mihomo-tui_${package_version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$package_root" "$output_dir/$deb_name"
(
  cd "$output_dir"
  sha256sum "$deb_name" >"$deb_name.sha256"
  chmod 644 "$deb_name.sha256"
)
echo "$output_dir/$deb_name"
