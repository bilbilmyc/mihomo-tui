#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ci_workflow="$repo_root/.github/workflows/ci.yml"
package_action="$repo_root/.github/actions/build-linux-packages/action.yml"
sync_workflow="$repo_root/.github/workflows/managed-core-sync.yml"

for file in "$ci_workflow" "$package_action" "$sync_workflow"; do
  [[ -f $file ]] || {
    echo "CI release policy file is missing: $file" >&2
    exit 1
  }
done

if rg -n 'actions/(upload|download)-artifact@|upload-artifact:' \
  "$ci_workflow" "$package_action" "$sync_workflow"; then
  echo "CI release builds must not depend on quota-limited workflow artifacts" >&2
  exit 1
fi

required_ci_contract=(
  'name: packages'
  'name: prepare-draft-release'
  'name: release-packages'
  "gh release create \"\$GITHUB_REF_NAME\""
  "gh release upload \"\$GITHUB_REF_NAME\" dist/* --clobber"
  'contents: write'
)

for contract in "${required_ci_contract[@]}"; do
  rg -Fq -- "$contract" "$ci_workflow" || {
    echo "CI release policy is missing: $contract" >&2
    exit 1
  }
done

for builder in build-native.sh build-deb.sh build-rpm.sh; do
  rg -Fq -- "./scripts/$builder" "$package_action" || {
    echo "package action does not build every release format: $builder" >&2
    exit 1
  }
done

echo "CI release policy tests passed"
