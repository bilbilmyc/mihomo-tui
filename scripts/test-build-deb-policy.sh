#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
builder="$repo_root/scripts/build-deb.sh"
temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
invalid_core="$temp_dir/mihomo.deb"
printf 'not an official Mihomo package\n' >"$invalid_core"

real_dpkg=$(command -v dpkg)
fake_bin="$temp_dir/fake-bin"
mkdir "$fake_bin"
{
  echo '#!/usr/bin/env bash'
  echo 'if [[ ${1:-} == --print-architecture ]]; then'
  echo '  printf "%s\n" "${MIHOMO_TUI_TEST_ARCH:?}"'
  echo '  exit 0'
  echo 'fi'
  printf 'exec %q "$@"\n' "$real_dpkg"
} >"$fake_bin/dpkg"
chmod 755 "$fake_bin/dpkg"

if "$builder" --architecture unsupported --core-deb "$invalid_core" >/dev/null 2>&1; then
  echo "Debian builder accepted an unsupported architecture" >&2
  exit 1
fi

native_arch=$(dpkg --print-architecture)
if [[ $native_arch == amd64 || $native_arch == arm64 ]]; then
  if "$builder" --architecture "$native_arch" --core-deb "$invalid_core" >/dev/null 2>&1; then
    echo "Debian builder accepted a core with the wrong SHA-256" >&2
    exit 1
  fi
fi

for architecture in amd64 arm64; do
  if output=$(
    MIHOMO_TUI_TEST_ARCH="$architecture" PATH="$fake_bin:$PATH" \
      "$builder" --architecture "$architecture" --core-deb "$invalid_core" 2>&1
  ); then
    echo "Debian builder accepted a core with the wrong SHA-256 for $architecture" >&2
    exit 1
  fi
  grep -F 'core deb SHA-256 mismatch' <<<"$output" >/dev/null || {
    echo "Debian builder rejected the $architecture manifest target before checksum validation" >&2
    exit 1
  }
done

echo "Debian bundle policy tests passed"
