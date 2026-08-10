#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 ]] || {
  echo "usage: sudo $0 PACKAGE.deb" >&2
  exit 2
}
[[ ${EUID:-$(id -u)} == 0 ]] || {
  echo "disposable-host package test must run as root" >&2
  exit 1
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
package=$1
manifest="$repo_root/managed-core.json"
for tool in dpkg dpkg-deb dpkg-query grep id jq ln readlink rm stat systemctl; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done
[[ -f $package && ! -L $package ]] || {
  echo "package is not a regular file: $package" >&2
  exit 1
}
[[ -d /run/systemd/system ]] || {
  echo "disposable-host package test requires a running systemd" >&2
  exit 1
}

if dpkg-query --show mihomo-tui >/dev/null 2>&1 || dpkg-query --show mihomo >/dev/null 2>&1; then
  echo "refusing to replace an existing Mihomo package" >&2
  exit 1
fi
for path in \
  /etc/mihomo-tui \
  /usr/bin/mihomo \
  /usr/bin/mihomo-tui \
  /usr/lib/mihomo-tui \
  /usr/lib/systemd/system/mihomo.service; do
  [[ ! -e $path && ! -L $path ]] || {
    echo "refusing to replace existing host path: $path" >&2
    exit 1
  }
done

core_tag=$(jq -er '.recommended' "$manifest")
architecture=$(dpkg-deb --field "$package" Architecture)
[[ $architecture == "$(dpkg --print-architecture)" ]]

cleanup() {
  dpkg --purge mihomo-tui >/dev/null 2>&1 || true
}
trap cleanup EXIT

dpkg --install "$package"
[[ $(dpkg-query --show --showformat='${Status}' mihomo-tui) == "install ok installed" ]]
[[ -x /usr/bin/mihomo-tui ]]
[[ ! -e /usr/bin/mihomo && ! -L /usr/bin/mihomo ]]
bundled_core="/usr/lib/mihomo-tui/bundled/$core_tag/mihomo"
managed_core="/usr/lib/mihomo-tui/cores/$core_tag/mihomo"
[[ -x $bundled_core ]]
[[ -x /usr/lib/mihomo-tui/cores/$core_tag/mihomo ]]
[[ $(readlink /usr/lib/mihomo-tui/current) == "cores/$core_tag" ]]
[[ -f /usr/lib/systemd/system/mihomo.service ]]
[[ $(stat -c '%i' "$bundled_core") == "$(stat -c '%i' "$managed_core")" ]]
package_files=$(dpkg-query --listfiles mihomo-tui)
grep -F "/usr/lib/mihomo-tui/bundled/$core_tag/mihomo" <<<"$package_files" >/dev/null
if grep -F "/usr/lib/mihomo-tui/cores/$core_tag/mihomo" <<<"$package_files" >/dev/null; then
  echo "managed rollback core is incorrectly owned by dpkg" >&2
  exit 1
fi

if systemctl is-enabled --quiet mihomo.service; then
  echo "package installation enabled mihomo.service" >&2
  exit 1
fi
if systemctl is-active --quiet mihomo.service; then
  echo "package installation started mihomo.service" >&2
  exit 1
fi

status_output=$(/usr/bin/mihomo-tui core status)
grep -F "active: $core_tag" <<<"$status_output" >/dev/null
grep -F "installed: $core_tag" <<<"$status_output" >/dev/null
grep -F "license: GPL-3.0" <<<"$status_output" >/dev/null

rm -f "$bundled_core"
[[ -x $managed_core ]]
"$managed_core" -v >/dev/null
ln -sfn cores/operator-selected /usr/lib/mihomo-tui/current
dpkg --install "$package"
[[ -x $bundled_core && -x $managed_core ]]
[[ $(readlink /usr/lib/mihomo-tui/current) == "cores/operator-selected" ]] || {
  echo "package upgrade replaced the operator-selected active core" >&2
  exit 1
}
ln -sfn "cores/$core_tag" /usr/lib/mihomo-tui/current

dpkg --purge mihomo-tui
trap - EXIT
for path in \
  /etc/mihomo-tui \
  /usr/bin/mihomo-tui \
  /usr/lib/mihomo-tui \
  /usr/lib/systemd/system/mihomo.service; do
  [[ ! -e $path && ! -L $path ]] || {
    echo "package purge left managed path behind: $path" >&2
    exit 1
  }
done

echo "Disposable-host Debian install verification passed: $package"
