#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
sync_script="$repo_root/scripts/sync-managed-core.sh"
manifest="$repo_root/managed-core.json"

recommended=$(jq -er '.recommended' "$manifest")
maximum=$(jq -er '.maximum_exclusive' "$manifest")

"$sync_script" --check-tag "$recommended"
if "$sync_script" --check-tag "$maximum" >/dev/null 2>&1; then
  echo "sync policy accepted the exclusive compatibility maximum" >&2
  exit 1
fi
if "$sync_script" --check-tag "v01.2.3" >/dev/null 2>&1; then
  echo "sync policy accepted a non-canonical tag" >&2
  exit 1
fi

echo "managed-core sync policy tests passed"
