#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 ]] || {
  echo "usage: sudo $0 PACKAGE.rpm" >&2
  exit 2
}
[[ ${EUID:-$(id -u)} == 0 ]] || {
  echo "disposable-host package test must run as root" >&2
  exit 1
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
package=$(realpath "$1")
manifest="$repo_root/managed-core.json"
for tool in dnf grep id jq ln readlink realpath rm rpm stat; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done
[[ -f $package && ! -L $package ]] || {
  echo "package is not a regular file: $package" >&2
  exit 1
}
[[ $(rpm -qp --queryformat '%{ARCH}' "$package") == "$(uname -m)" ]]

if rpm -q mihomo-tui >/dev/null 2>&1 || rpm -q mihomo >/dev/null 2>&1; then
  echo "refusing to replace an existing Mihomo package" >&2
  exit 1
fi
for path in \
  /etc/mihomo-tui \
  /usr/bin/mihomo \
  /usr/local/bin/mihomo \
  /usr/bin/mihomo-tui \
  /usr/lib/mihomo-tui \
  /usr/lib/systemd/system/mihomo.service; do
  [[ ! -e $path && ! -L $path ]] || {
    echo "refusing to replace existing host path: $path" >&2
    exit 1
  }
done

core_tag=$(jq -er '.recommended' "$manifest")
conflicting_core=/usr/local/bin/mihomo
cleanup() {
  rm -f "$conflicting_core"
  rpm -e mihomo-tui >/dev/null 2>&1 || true
}
trap cleanup EXIT

dnf install -y "$package"
[[ $(rpm -q --queryformat '%{NAME}' mihomo-tui) == mihomo-tui ]]
[[ -x /usr/bin/mihomo-tui ]]
[[ ! -e /usr/bin/mihomo && ! -L /usr/bin/mihomo ]]
bundled_core="/usr/lib/mihomo-tui/bundled/$core_tag/mihomo"
managed_core="/usr/lib/mihomo-tui/cores/$core_tag/mihomo"
[[ -x $bundled_core && -x $managed_core ]]
[[ $(readlink /usr/lib/mihomo-tui/current) == "cores/$core_tag" ]]
[[ -f /usr/lib/systemd/system/mihomo.service ]]
[[ $(stat -c '%i' "$bundled_core") == "$(stat -c '%i' "$managed_core")" ]]
package_files=$(rpm -ql mihomo-tui)
grep -F "/usr/lib/mihomo-tui/bundled/$core_tag/mihomo" <<<"$package_files" >/dev/null
if grep -F "/usr/lib/mihomo-tui/cores/$core_tag/mihomo" <<<"$package_files" >/dev/null; then
  echo "managed rollback core is incorrectly owned by RPM" >&2
  exit 1
fi

status_output=$(/usr/bin/mihomo-tui core status)
grep -F "active: $core_tag" <<<"$status_output" >/dev/null
grep -F "installed: $core_tag" <<<"$status_output" >/dev/null

rm -f "$bundled_core"
[[ -x $managed_core ]]
ln -sfn cores/operator-selected /usr/lib/mihomo-tui/current
dnf reinstall -y "$package"
[[ -x $bundled_core && -x $managed_core ]]
[[ $(readlink /usr/lib/mihomo-tui/current) == "cores/operator-selected" ]] || {
  echo "package upgrade replaced the operator-selected active core" >&2
  exit 1
}
ln -sfn "cores/$core_tag" /usr/lib/mihomo-tui/current

rpm -e mihomo-tui
for path in \
  /etc/mihomo-tui \
  /usr/bin/mihomo-tui \
  /usr/lib/mihomo-tui \
  /usr/lib/systemd/system/mihomo.service; do
  [[ ! -e $path && ! -L $path ]] || {
    echo "package removal left managed path behind: $path" >&2
    exit 1
  }
done

install -m 755 /bin/true "$conflicting_core"
if install_output=$(dnf install -y "$package" 2>&1); then
  echo "package installation accepted an unmanaged core path" >&2
  exit 1
fi
grep -F "refusing to replace unmanaged host path: $conflicting_core" <<<"$install_output" >/dev/null
[[ -x $conflicting_core ]]
[[ ! -e /usr/bin/mihomo-tui && ! -L /usr/bin/mihomo-tui ]]

cleanup
trap - EXIT
echo "Disposable-host RPM install verification passed: $package"
