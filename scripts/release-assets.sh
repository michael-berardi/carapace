#!/bin/sh
# Builds the GitHub release assets: the npm client tarball and its SHA-256 file.
# Usage: scripts/release-assets.sh [out-dir]   (default: target/release-assets)
# Needs Node 20+. Writes only the output folder and git-ignored build output.
set -eu
export DO_NOT_TRACK=1
export npm_config_update_notifier=false npm_config_audit=false npm_config_fund=false npm_config_telemetry_enabled=false
cd "$(dirname "$0")/.."

version=$(node -p 'require("./packages/client/package.json").version')
workspace=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
if [ "$version" != "$workspace" ]; then
  echo "release-assets: @carapace/client is $version but the Cargo workspace is $workspace" >&2
  exit 1
fi

out=${1:-target/release-assets}
mkdir -p "$out"
out=$(cd "$out" && pwd)
name="carapace-client-$version.tgz"
rm -f "$out/$name" "$out/$name.sha256"

(
  cd packages/client
  rm -rf dist
  npm ci --silent
  npm run -s build
  npm pack --silent --pack-destination "$out" >/dev/null
)
if [ ! -f "$out/$name" ]; then
  echo "release-assets: npm pack did not write $out/$name" >&2
  exit 1
fi
(cd "$out" && shasum -a 256 "$name" > "$name.sha256")
echo "$out/$name"
echo "$out/$name.sha256"
