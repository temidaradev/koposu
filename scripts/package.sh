#!/usr/bin/env bash
# Build the component and print the marketplace listing for it.
#
#   scripts/package.sh [tag]
#
# Leaves dist/kopuz-plugin.toml and dist/plugin.wasm, the two files a release
# carries, and prints the entry for Kopuz-org/plugins' index.json. The tag
# (v<version> by default) names the release the files are downloaded from.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' kopuz-plugin.toml | head -n1)
crate_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
if [ "$version" != "$crate_version" ]; then
  echo "kopuz-plugin.toml is $version but Cargo.toml is $crate_version" >&2
  exit 1
fi
tag=${1:-v$version}

# Host linker flags (a dev shell's -fuse-ld=lld) do not apply to wasm.
RUSTFLAGS= cargo build --locked --release --target wasm32-wasip2

mkdir -p dist
cp kopuz-plugin.toml dist/kopuz-plugin.toml
cp target/wasm32-wasip2/release/koposu.wasm dist/plugin.wasm

sha() { sha256sum "$1" | cut -d' ' -f1; }
base="https://github.com/temidaradev/koposu/releases/download/$tag"
cat <<JSON
{
  "id": "koposu",
  "name": "koposu",
  "version": "$version",
  "api": "0.1",
  "description": "Your osu!lazer and osu!stable song libraries, played from the folders they are in.",
  "author": "temidaradev",
  "homepage": "https://github.com/temidaradev/koposu",
  "services": ["osu"],
  "network": [],
  "folders": "read",
  "manifest": { "url": "$base/kopuz-plugin.toml", "sha256": "$(sha dist/kopuz-plugin.toml)" },
  "component": { "url": "$base/plugin.wasm", "sha256": "$(sha dist/plugin.wasm)" }
}
JSON
