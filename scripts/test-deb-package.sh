#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 ]] || {
  echo "usage: $0 PACKAGE.deb" >&2
  exit 2
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$repo_root/managed-core.json"
package=$1
for tool in basename dirname dpkg dpkg-deb grep jq readlink sed sh sha256sum stat systemd-analyze; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done
[[ -f $package && ! -L $package ]] || {
  echo "package is not a regular file: $package" >&2
  exit 1
}

architecture=$(dpkg-deb --field "$package" Architecture)
core_tag=$(jq -er '.recommended' "$manifest")
license_sha256=$(jq -er '.license.sha256' "$manifest")
[[ $(dpkg-deb --field "$package" Package) == mihomo-tui ]]
[[ $(dpkg-deb --field "$package" Conflicts) == mihomo ]]
[[ $architecture == amd64 || $architecture == arm64 ]]
depends=$(dpkg-deb --field "$package" Depends)
grep -Eq '(^|, )libc6( \([^)]*\))?(,|$)' <<<"$depends"
grep -Eq '(^|, )libgcc-s1( \([^)]*\))?(,|$)' <<<"$depends"

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
root="$temp_dir/root"
control="$temp_dir/control"
dpkg-deb --extract "$package" "$root"
dpkg-deb --control "$package" "$control"

[[ -f $root/usr/bin/mihomo-tui && -x $root/usr/bin/mihomo-tui && ! -L $root/usr/bin/mihomo-tui ]]
[[ ! -e $root/usr/bin/mihomo && ! -L $root/usr/bin/mihomo ]]
bundled_binary="$root/usr/lib/mihomo-tui/bundled/$core_tag/mihomo"
[[ -f $bundled_binary && -x $bundled_binary && ! -L $bundled_binary ]]
[[ ! -e $root/usr/lib/mihomo-tui/cores/$core_tag/mihomo ]]
[[ ! -e $root/usr/lib/mihomo-tui/current && ! -L $root/usr/lib/mihomo-tui/current ]]
grep -F "default_core='$core_tag'" "$control/postinst" >/dev/null
grep -F 'bundled_root=$managed_root/bundled' "$control/postinst" >/dev/null
grep -F 'bundled_core=$bundled_version_root/mihomo' "$control/postinst" >/dev/null
for protected_path in \
  /usr/bin/mihomo \
  /usr/local/bin/mihomo \
  /usr/bin/mihomo-tui \
  /usr/local/bin/mihomo-tui \
  /usr/lib/mihomo-tui \
  /etc/systemd/system/mihomo.service \
  /etc/systemd/system/mihomo.service.d \
  /etc/systemd/system/multi-user.target.wants/mihomo.service \
  /run/systemd/system/mihomo.service \
  /run/systemd/system/mihomo.service.d \
  /usr/lib/systemd/system/mihomo.service \
  /lib/systemd/system/mihomo.service; do
  grep -F "$protected_path" "$control/preinst" >/dev/null
done
grep -F "ExecStart=/usr/lib/mihomo-tui/current/mihomo -d /etc/mihomo-tui" "$root/usr/lib/systemd/system/mihomo.service" >/dev/null
grep -F "Source: https://github.com/MetaCubeX/mihomo/tree/$core_tag" "$root/usr/share/doc/mihomo-tui/Mihomo-NOTICE" >/dev/null
actual_license_sha256=$(sha256sum "$root/usr/share/doc/mihomo-tui/Mihomo-LICENSE")
actual_license_sha256=${actual_license_sha256%% *}
[[ $actual_license_sha256 == "$license_sha256" ]]
for document in Mihomo-LICENSE Mihomo-NOTICE copyright; do
  [[ $(stat -c '%a' "$root/usr/share/doc/mihomo-tui/$document") == 644 ]]
done

for script in preinst postinst prerm postrm; do
  sh -n "$control/$script"
done
verification_unit="$temp_dir/mihomo-verify.service"
sed "s|/usr/lib/mihomo-tui/current/mihomo|$bundled_binary|" \
  "$root/usr/lib/systemd/system/mihomo.service" >"$verification_unit"
SYSTEMD_UNIT_PATH="$temp_dir:/usr/lib/systemd/system:/lib/systemd/system" \
  systemd-analyze verify "$verification_unit"

if [[ $(dpkg --print-architecture) == "$architecture" ]]; then
  "$root/usr/bin/mihomo-tui" --version >/dev/null
  version_output=$("$bundled_binary" -v)
  core_tag_pattern=${core_tag//./\\.}
  [[ $version_output =~ (^|[[:space:]])$core_tag_pattern([[:space:]]|$) ]]
fi

checksum_file="$package.sha256"
[[ -f $checksum_file ]]
[[ $(stat -c '%a' "$checksum_file") == 644 ]]
(
  cd "$(dirname "$package")"
  sha256sum --check "$(basename "$checksum_file")"
)
echo "Debian package verification passed: $package"
