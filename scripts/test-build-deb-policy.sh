#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
builder="$repo_root/scripts/build-deb.sh"
temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
invalid_core="$temp_dir/mihomo.deb"
printf 'not an official Mihomo package\n' >"$invalid_core"

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

echo "Debian bundle policy tests passed"
