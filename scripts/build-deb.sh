#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/lib/release.sh"
RELEASE_REPO_ROOT=$repo_root
RELEASE_MANIFEST="$repo_root/managed-core.json"

architecture=$(dpkg --print-architecture)
core_deb=""
license_file=""
binary="$repo_root/target/release/mihomo-tui"
binary_explicit=false
output_dir="$repo_root/dist"

usage() {
  echo "usage: $0 [--architecture amd64|arm64] [--core-deb PATH] [--license-file PATH] [--binary PATH] [--output-dir DIR]" >&2
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --architecture) architecture=${2:?missing architecture}; shift 2 ;;
    --core-deb) core_deb=${2:?missing core deb path}; shift 2 ;;
    --license-file) license_file=${2:?missing license file path}; shift 2 ;;
    --binary) binary=${2:?missing binary path}; binary_explicit=true; shift 2 ;;
    --output-dir) output_dir=${2:?missing output directory}; shift 2 ;;
    *) usage; exit 2 ;;
  esac
done

release_require_tools awk cargo chmod curl dpkg dpkg-deb dpkg-shlibdeps du file install jq mkdir mktemp realpath sed sha256sum stat uname
release_load_target "$architecture"
[[ $architecture == "$RELEASE_DEB_ARCH" ]] || {
  echo "Debian artifacts use amd64 or arm64 architecture names" >&2
  exit 1
}
native_deb_arch=$(dpkg --print-architecture)
[[ $native_deb_arch == "$RELEASE_DEB_ARCH" ]] || {
  echo "package builds must run natively on $RELEASE_DEB_ARCH (host is $native_deb_arch)" >&2
  exit 1
}

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
release_prepare_core "$core_deb" "$temp_dir"
release_read_app_version
release_require_native_arch
release_prepare_binary "$binary" "$binary_explicit"
release_prepare_license "$license_file" "$temp_dir"

shlibs_workspace="$temp_dir/shlibs"
mkdir -p "$shlibs_workspace/debian/mihomo-tui/DEBIAN"
install -D -m 755 "$RELEASE_BINARY" "$shlibs_workspace/debian/mihomo-tui/usr/bin/mihomo-tui"
{
  echo "Source: mihomo-tui"
  echo "Section: net"
  echo "Priority: optional"
  echo "Maintainer: bilbilmyc <1361998242@qq.com>"
  echo
  echo "Package: mihomo-tui"
  echo "Architecture: any"
  echo "Depends: \${shlibs:Depends}"
  echo "Description: temporary metadata for dependency calculation"
} >"$shlibs_workspace/debian/control"
shlibs_assignment=$(cd "$shlibs_workspace" && dpkg-shlibdeps -O debian/mihomo-tui/usr/bin/mihomo-tui)
shared_dependencies=${shlibs_assignment#shlibs:Depends=}
[[ -n $shared_dependencies && $shared_dependencies != "$shlibs_assignment" ]] || {
  echo "could not determine mihomo-tui shared library dependencies" >&2
  exit 1
}

package_version="${RELEASE_APP_VERSION}+mihomo${RELEASE_CORE_DEB_VERSION}-1"
package_root="$temp_dir/package"
install -d -m 755 \
  "$package_root/DEBIAN" \
  "$package_root/etc/mihomo-tui" \
  "$package_root/usr/bin" \
  "$package_root/usr/lib/mihomo-tui/bundled/$RELEASE_CORE_TAG" \
  "$package_root/usr/lib/systemd/system" \
  "$package_root/usr/share/doc/mihomo-tui"
install -m 755 "$RELEASE_BINARY" "$package_root/usr/bin/mihomo-tui"
install -m 755 "$RELEASE_CORE_BINARY" "$package_root/usr/lib/mihomo-tui/bundled/$RELEASE_CORE_TAG/mihomo"
install -m 644 "$repo_root/packaging/debian/mihomo.service" "$package_root/usr/lib/systemd/system/mihomo.service"
install -m 644 "$repo_root/docs/server-guide.md" "$package_root/usr/share/doc/mihomo-tui/server-guide.md"
install -m 644 "$repo_root/LICENSE" "$package_root/usr/share/doc/mihomo-tui/mihomo-tui-LICENSE"
install -m 644 "$RELEASE_LICENSE_FILE" "$package_root/usr/share/doc/mihomo-tui/Mihomo-LICENSE"
release_write_notice "$package_root/usr/share/doc/mihomo-tui/Mihomo-NOTICE"
release_write_copyright "$package_root/usr/share/doc/mihomo-tui/copyright"

sed "s|@CORE_TAG@|$RELEASE_CORE_TAG|g" "$repo_root/packaging/debian/postinst" >"$package_root/DEBIAN/postinst"
chmod 755 "$package_root/DEBIAN/postinst"
install -m 755 "$repo_root/packaging/debian/preinst" "$package_root/DEBIAN/preinst"
install -m 755 "$repo_root/packaging/debian/prerm" "$package_root/DEBIAN/prerm"
install -m 755 "$repo_root/packaging/debian/postrm" "$package_root/DEBIAN/postrm"
installed_size=$(du -sk "$package_root/usr" "$package_root/etc" | awk '{ total += $1 } END { print total }')
{
  echo "Package: mihomo-tui"
  echo "Version: $package_version"
  echo "Architecture: $RELEASE_DEB_ARCH"
  echo "Maintainer: bilbilmyc <1361998242@qq.com>"
  echo "Section: net"
  echo "Priority: optional"
  echo "Installed-Size: $installed_size"
  echo "Depends: $shared_dependencies, ca-certificates, systemd"
  echo "Recommends: iproute2"
  echo "Conflicts: mihomo"
  echo "Description: SSH-friendly terminal control center with a pinned Mihomo core"
  echo " Ships mihomo-tui and the reviewed official Mihomo $RELEASE_CORE_TAG core in a"
  echo " versioned layout. Installation does not automatically start the service."
} >"$package_root/DEBIAN/control"

mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
artifact="$output_dir/mihomo-tui_${package_version}_${RELEASE_DEB_ARCH}.deb"
dpkg-deb --build --root-owner-group "$package_root" "$artifact"
release_write_checksum "$artifact"
echo "$artifact"
