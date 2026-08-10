#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$repo_root/managed-core.json"
repository="MetaCubeX/mihomo"
api_url="https://api.github.com/repos/$repository/releases/latest"

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required tool is missing: $1" >&2
    exit 1
  }
}

validate_tag() {
  local tag=$1
  [[ $tag =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
    echo "latest Mihomo tag is not canonical vMAJOR.MINOR.PATCH: $tag" >&2
    return 1
  }

  local version=${tag#v}
  local minimum maximum
  minimum=$(jq -er '.minimum_supported' "$manifest")
  maximum=$(jq -er '.maximum_exclusive' "$manifest")
  dpkg --compare-versions "$version" ge "${minimum#v}" || {
    echo "$tag is below the reviewed compatibility range ($minimum to $maximum)" >&2
    return 1
  }
  dpkg --compare-versions "$version" lt "${maximum#v}" || {
    echo "$tag is outside the reviewed compatibility range ($minimum to $maximum)" >&2
    return 1
  }
}

for tool in jq dpkg dpkg-deb curl sha256sum mktemp; do
  require_tool "$tool"
done

jq -e '.schema == 1 and .repository == "MetaCubeX/mihomo"' "$manifest" >/dev/null || {
  echo "managed-core.json has an unsupported schema or repository" >&2
  exit 1
}

if [[ ${1:-} == "--check-tag" ]]; then
  [[ $# == 2 ]] || {
    echo "usage: $0 --check-tag vMAJOR.MINOR.PATCH" >&2
    exit 2
  }
  validate_tag "$2"
  exit 0
fi
[[ $# == 0 ]] || {
  echo "usage: $0 [--check-tag vMAJOR.MINOR.PATCH]" >&2
  exit 2
}

temp_dir=$(mktemp -d)
trap 'rm -rf -- "$temp_dir"' EXIT
release_json="$temp_dir/release.json"
curl --fail --silent --show-error --location \
  --retry 3 --retry-all-errors \
  --proto '=https' --proto-redir '=https' \
  --user-agent "mihomo-tui-managed-core-sync" \
  --output "$release_json" "$api_url"

jq -e '.draft == false and .prerelease == false' "$release_json" >/dev/null || {
  echo "GitHub latest release is a draft or prerelease" >&2
  exit 1
}
tag=$(jq -er '.tag_name' "$release_json")
validate_tag "$tag"
deb_version=${tag#v}

asset_url() {
  local name=$1
  jq -er --arg name "$name" '
    [.assets[] | select(.name == $name)]
    | if length == 1 then .[0].browser_download_url
      else error("expected exactly one release asset named " + $name)
      end
  ' "$release_json"
}

fetch_package() {
  local rust_arch=$1
  local deb_arch=$2
  local asset=$3
  local output="$temp_dir/$asset"
  local url
  url=$(asset_url "$asset")
  [[ $url == "https://github.com/$repository/releases/download/$tag/$asset" ]] || {
    echo "unexpected release asset URL: $url" >&2
    return 1
  }
  curl --fail --silent --show-error --location \
    --retry 3 --retry-all-errors \
    --proto '=https' --proto-redir '=https' \
    --user-agent "mihomo-tui-managed-core-sync" \
    --output "$output" "$url"

  local package_name package_version package_architecture
  package_name=$(dpkg-deb --field "$output" Package)
  package_version=$(dpkg-deb --field "$output" Version)
  package_architecture=$(dpkg-deb --field "$output" Architecture)
  [[ $package_name == "mihomo" ]] || {
    echo "$asset has unexpected Package: $package_name" >&2
    return 1
  }
  [[ $package_version == "$deb_version" ]] || {
    echo "$asset has unexpected Version: $package_version" >&2
    return 1
  }
  [[ $package_architecture == "$deb_arch" ]] || {
    echo "$asset has unexpected Architecture: $package_architecture" >&2
    return 1
  }

  local sha256
  sha256=$(sha256sum "$output")
  sha256=${sha256%% *}
  jq -n \
    --arg os linux \
    --arg arch "$rust_arch" \
    --arg asset "$asset" \
    --arg sha256 "$sha256" \
    --arg deb_arch "$deb_arch" \
    --arg deb_version "$deb_version" \
    '{os:$os,arch:$arch,asset:$asset,sha256:$sha256,deb_arch:$deb_arch,deb_version:$deb_version}'
}

amd64_asset="mihomo-linux-amd64-v1-$tag.deb"
arm64_asset="mihomo-linux-arm64-$tag.deb"
amd64_package=$(fetch_package x86_64 amd64 "$amd64_asset")
arm64_package=$(fetch_package aarch64 arm64 "$arm64_asset")

jq \
  --arg recommended "$tag" \
  --argjson amd64 "$amd64_package" \
  --argjson arm64 "$arm64_package" \
  '.recommended = $recommended | .packages = [$amd64, $arm64]' \
  "$manifest" >"$temp_dir/managed-core.json"
jq -e . "$temp_dir/managed-core.json" >/dev/null
mv "$temp_dir/managed-core.json" "$manifest"
echo "managed-core.json now recommends $tag"
