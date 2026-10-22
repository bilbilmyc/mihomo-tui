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

# Only the branch packages job uploads optional-lifetime CI downloads. The shared
# build action and tag release flow must remain independent of artifact storage.
if rg -n 'actions/(upload|download)-artifact@|upload-artifact:' \
  "$package_action" "$sync_workflow"; then
  echo "Shared release builds must not depend on workflow artifact storage" >&2
  exit 1
fi

if rg -n 'actions/download-artifact@' "$ci_workflow"; then
  echo "Tag releases must download their verified files directly from the draft Release" >&2
  exit 1
fi

required_ci_contract=(
  'name: packages'
  'name: prepare-draft-release'
  'name: release-packages'
  "gh release create \"\$GITHUB_REF_NAME\""
  "gh release upload \"\$GITHUB_REF_NAME\" dist/* --clobber"
  "gh release view \"\$GITHUB_REF_NAME\" --repo \"\$GITHUB_REPOSITORY\" --json isDraft --jq .isDraft"
  "gh release download \"\$GITHUB_REF_NAME\" --repo \"\$GITHUB_REPOSITORY\" --dir release-dist"
  'contents: write'
  'needs: [quality, security-audit]'
  'name: Upload verified branch packages'
  'actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9'
  "name: mihomo-tui-linux-\${{ matrix.rust-architecture }}-\${{ github.sha }}"
  'retention-days: 7'
  'if-no-files-found: error'
  'include-hidden-files: false'
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
