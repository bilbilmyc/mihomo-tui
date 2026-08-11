#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
rpm_builder="$repo_root/scripts/build-rpm.sh"
native_builder="$repo_root/scripts/build-native.sh"

for builder in "$rpm_builder" "$native_builder"; do
  [[ -x $builder ]] || {
    echo "release builder is missing or not executable: $builder" >&2
    exit 1
  }
done

if "$rpm_builder" --architecture unsupported >/dev/null 2>&1; then
  echo "RPM builder accepted an unsupported architecture" >&2
  exit 1
fi

if "$native_builder" --architecture unsupported >/dev/null 2>&1; then
  echo "native builder accepted an unsupported architecture" >&2
  exit 1
fi

native_arch=$(uname -m)
if [[ $native_arch == x86_64 || $native_arch == aarch64 ]]; then
  temp_dir=$(mktemp -d)
  trap 'rm -rf -- "$temp_dir"' EXIT
  fake_binary="$temp_dir/mihomo-tui"
  printf '#!/bin/sh\nprintf "mihomo-tui 0.0.0\\n"\n' >"$fake_binary"
  chmod 755 "$fake_binary"

  if "$native_builder" \
    --architecture "$native_arch" \
    --binary "$fake_binary" \
    --output-dir "$temp_dir/out" >/dev/null 2>&1; then
    echo "native builder accepted a script with the wrong version" >&2
    exit 1
  fi
fi

echo "Release builder policy tests passed"
