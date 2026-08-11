#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/lib/release.sh"
RELEASE_REPO_ROOT=$repo_root
RELEASE_MANIFEST="$repo_root/managed-core.json"

architecture=$(uname -m)
binary="$repo_root/target/release/mihomo-tui"
binary_explicit=false
output_dir="$repo_root/dist"

usage() {
  echo "usage: $0 [--architecture x86_64|aarch64] [--binary PATH] [--output-dir DIR]" >&2
}

while [[ $# -gt 0 ]]; do
  case $1 in
    --architecture) architecture=${2:?missing architecture}; shift 2 ;;
    --binary) binary=${2:?missing binary path}; binary_explicit=true; shift 2 ;;
    --output-dir) output_dir=${2:?missing output directory}; shift 2 ;;
    *) usage; exit 2 ;;
  esac
done

release_require_tools cargo file install jq mkdir realpath sha256sum uname
release_load_target "$architecture"
[[ $architecture == "$RELEASE_RUST_ARCH" ]] || {
  echo "native artifacts use x86_64 or aarch64 architecture names" >&2
  exit 1
}
release_require_native_arch
release_read_app_version
release_prepare_binary "$binary" "$binary_explicit"

mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
artifact="$output_dir/mihomo-tui-${RELEASE_APP_VERSION}-linux-${RELEASE_RUST_ARCH}"
install -m 755 "$RELEASE_BINARY" "$artifact"
release_write_checksum "$artifact"
echo "$artifact"
