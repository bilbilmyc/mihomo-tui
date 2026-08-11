#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 ]] || {
  echo "usage: $0 NATIVE_BINARY" >&2
  exit 2
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary=$1
for tool in basename cargo dirname file jq sha256sum stat; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required tool is missing: $tool" >&2
    exit 1
  }
done
[[ -f $binary && -x $binary && ! -L $binary ]] || {
  echo "native artifact is not a regular executable: $binary" >&2
  exit 1
}

app_version=$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --no-deps --format-version 1 |
  jq -er '.packages[] | select(.name == "mihomo-tui") | .version')
case $(basename "$binary") in
  "mihomo-tui-${app_version}-linux-x86_64") architecture=x86_64 ;;
  "mihomo-tui-${app_version}-linux-aarch64") architecture=aarch64 ;;
  *)
    echo "native artifact has an unexpected name: $(basename "$binary")" >&2
    exit 1
    ;;
esac

description=$(file -b "$binary")
[[ $description == *ELF* ]] || {
  echo "native artifact is not an ELF binary: $description" >&2
  exit 1
}
case $architecture in
  x86_64) [[ $description == *"x86-64"* ]] ;;
  aarch64) [[ $description == *"aarch64"* || $description == *"ARM64"* ]] ;;
esac || {
  echo "native artifact architecture mismatch: $description" >&2
  exit 1
}

if [[ $(uname -m) == "$architecture" ]]; then
  [[ $("$binary" --version) == "mihomo-tui $app_version" ]]
  help_output=$("$binary" -h)
  grep -F '快速开始' <<<"$help_output" >/dev/null
fi

checksum_file="$binary.sha256"
[[ -f $checksum_file && ! -L $checksum_file ]]
[[ $(stat -c '%a' "$checksum_file") == 644 ]]
(
  cd "$(dirname "$binary")"
  sha256sum --check "$(basename "$checksum_file")"
)

echo "Native binary verification passed: $binary"
