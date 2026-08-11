#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/lib/release.sh"
RELEASE_REPO_ROOT=$repo_root
RELEASE_MANIFEST="$repo_root/managed-core.json"

architecture=$(uname -m)
core_deb=""
license_file=""
binary="$repo_root/target/release/mihomo-tui"
binary_explicit=false
output_dir="$repo_root/dist"

usage() {
  echo "usage: $0 [--architecture x86_64|aarch64] [--core-deb PATH] [--license-file PATH] [--binary PATH] [--output-dir DIR]" >&2
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

release_require_tools cargo chmod cp curl dpkg-deb file install jq mkdir mktemp realpath rpmbuild sed sha256sum stat uname
release_load_target "$architecture"
[[ $architecture == "$RELEASE_RPM_ARCH" ]] || {
  echo "RPM artifacts use x86_64 or aarch64 architecture names" >&2
  exit 1
}
release_require_native_arch

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
release_prepare_core "$core_deb" "$temp_dir"
release_read_app_version
release_prepare_binary "$binary" "$binary_explicit"
release_prepare_license "$license_file" "$temp_dir"

topdir="$temp_dir/rpmbuild"
rpm_temp="$temp_dir/rpm-tmp"
mkdir -p "$rpm_temp" "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"
install -m 755 "$RELEASE_BINARY" "$topdir/SOURCES/mihomo-tui"
install -m 755 "$RELEASE_CORE_BINARY" "$topdir/SOURCES/mihomo"
install -m 644 "$repo_root/packaging/debian/mihomo.service" "$topdir/SOURCES/mihomo.service"
install -m 644 "$repo_root/docs/server-guide.md" "$topdir/SOURCES/server-guide.md"
install -m 644 "$repo_root/LICENSE" "$topdir/SOURCES/mihomo-tui-LICENSE"
install -m 644 "$RELEASE_LICENSE_FILE" "$topdir/SOURCES/Mihomo-LICENSE"
release_write_notice "$topdir/SOURCES/Mihomo-NOTICE"
sed \
  -e "s|@APP_VERSION@|$RELEASE_APP_VERSION|g" \
  -e "s|@CORE_VERSION@|$RELEASE_CORE_DEB_VERSION|g" \
  -e "s|@CORE_TAG@|$RELEASE_CORE_TAG|g" \
  -e "s|@RPM_ARCH@|$RELEASE_RPM_ARCH|g" \
  "$repo_root/packaging/rpm/mihomo-tui.spec.in" >"$topdir/SPECS/mihomo-tui.spec"

rpmbuild -bb \
  --define "_topdir $topdir" \
  --define "_tmppath $rpm_temp" \
  --define "_build_id_links none" \
  --target "$RELEASE_RPM_ARCH" \
  "$topdir/SPECS/mihomo-tui.spec"

rpm_paths=("$topdir/RPMS/$RELEASE_RPM_ARCH/"*.rpm)
[[ ${#rpm_paths[@]} == 1 && -f ${rpm_paths[0]} ]] || {
  echo "RPM builder did not produce exactly one package" >&2
  exit 1
}
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
artifact="$output_dir/$(basename "${rpm_paths[0]}")"
install -m 644 "${rpm_paths[0]}" "$artifact"
release_write_checksum "$artifact"
echo "$artifact"
