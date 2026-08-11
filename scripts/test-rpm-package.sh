#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 ]] || {
  echo "usage: $0 PACKAGE.rpm" >&2
  exit 2
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$repo_root/managed-core.json"
package=$1
for tool in basename cpio dirname grep jq realpath rpm rpm2cpio sed sha256sum stat systemd-analyze; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done
[[ -f $package && ! -L $package ]] || {
  echo "package is not a regular file: $package" >&2
  exit 1
}
package=$(realpath "$package")

core_tag=$(jq -er '.recommended' "$manifest")
license_sha256=$(jq -er '.license.sha256' "$manifest")
[[ $(rpm -qp --queryformat '%{NAME}' "$package") == mihomo-tui ]]
[[ $(rpm -qp --queryformat '%{CONFLICTNAME}' "$package") == mihomo ]]
[[ $(rpm -qp --queryformat '%{LICENSE}' "$package") == 'MIT AND GPL-3.0-only' ]]
[[ $(rpm -qp --queryformat '%{PACKAGER}' "$package") == 'bilbilmyc <1361998242@qq.com>' ]]
architecture=$(rpm -qp --queryformat '%{ARCH}' "$package")
[[ $architecture == x86_64 || $architecture == aarch64 ]]
requires=$(rpm -qp --requires "$package")
grep -Fx 'ca-certificates' <<<"$requires" >/dev/null
grep -Fx 'systemd' <<<"$requires" >/dev/null

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
root="$temp_dir/root"
mkdir "$root"
(
  cd "$root"
  rpm2cpio "$package" | cpio --quiet -idm --no-absolute-filenames
)

tui_binary="$root/usr/bin/mihomo-tui"
bundled_binary="$root/usr/lib/mihomo-tui/bundled/$core_tag/mihomo"
unit="$root/usr/lib/systemd/system/mihomo.service"
[[ -f $tui_binary && -x $tui_binary && ! -L $tui_binary ]]
[[ -f $bundled_binary && -x $bundled_binary && ! -L $bundled_binary ]]
[[ ! -e $root/usr/bin/mihomo && ! -L $root/usr/bin/mihomo ]]
[[ ! -e $root/usr/lib/mihomo-tui/cores/$core_tag/mihomo ]]
[[ ! -e $root/usr/lib/mihomo-tui/current && ! -L $root/usr/lib/mihomo-tui/current ]]
grep -F 'ExecStart=/usr/lib/mihomo-tui/current/mihomo -d /etc/mihomo-tui' "$unit" >/dev/null
grep -F "Source: https://github.com/MetaCubeX/mihomo/tree/$core_tag" \
  "$root/usr/share/doc/mihomo-tui/Mihomo-NOTICE" >/dev/null
actual_license_sha256=$(sha256sum "$root/usr/share/doc/mihomo-tui/Mihomo-LICENSE")
actual_license_sha256=${actual_license_sha256%% *}
[[ $actual_license_sha256 == "$license_sha256" ]]
project_license="$root/usr/share/doc/mihomo-tui/mihomo-tui-LICENSE"
project_license_sha256=$(sha256sum "$project_license")
project_license_sha256=${project_license_sha256%% *}
expected_project_license_sha256=$(sha256sum "$repo_root/LICENSE")
expected_project_license_sha256=${expected_project_license_sha256%% *}
[[ $project_license_sha256 == "$expected_project_license_sha256" ]]
[[ -f $root/usr/share/doc/mihomo-tui/server-guide.md ]]

scripts=$(rpm -qp --scripts "$package")
grep -F "default_core='$core_tag'" <<<"$scripts" >/dev/null
grep -F 'refusing to replace unmanaged host path' <<<"$scripts" >/dev/null
grep -F 'systemctl disable --now mihomo.service' <<<"$scripts" >/dev/null
grep -F 'rm -rf -- /usr/lib/mihomo-tui/cores' <<<"$scripts" >/dev/null
grep -F 'rm -rf -- /usr/lib/mihomo-tui/bundled' <<<"$scripts" >/dev/null

verification_unit="$temp_dir/mihomo-verify.service"
sed "s|/usr/lib/mihomo-tui/current/mihomo|$bundled_binary|" "$unit" >"$verification_unit"
SYSTEMD_UNIT_PATH="$temp_dir:/usr/lib/systemd/system:/lib/systemd/system" \
  systemd-analyze verify "$verification_unit"

host_arch=$(uname -m)
if [[ $host_arch == "$architecture" ]]; then
  [[ $("$tui_binary" --version) == "mihomo-tui $(rpm -qp --queryformat '%{VERSION}' "$package")" ]]
  "$bundled_binary" -v | grep -F "$core_tag" >/dev/null
fi

checksum_file="$package.sha256"
[[ -f $checksum_file && ! -L $checksum_file ]]
[[ $(stat -c '%a' "$checksum_file") == 644 ]]
(
  cd "$(dirname "$package")"
  sha256sum --check "$(basename "$checksum_file")"
)

echo "RPM package verification passed: $package"
