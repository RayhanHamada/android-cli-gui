#!/bin/bash
# Assemble a launchable macOS .app bundle around the release binary.
#
# Usage: make-app.sh <binary> <version> <output.app>
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <binary> <version> <output.app>" >&2
  exit 2
fi

binary=$1
version=$2
app=$3

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "make-app.sh requires macOS (sips, iconutil, codesign)." >&2
  exit 1
fi
if [[ ! -x "$binary" ]]; then
  echo "Binary not found or not executable: $binary" >&2
  exit 1
fi
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Version must be a semantic version, got: $version" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
contents="$app/Contents"
resources="$contents/Resources"

rm -rf "$app"
mkdir -p "$contents/MacOS" "$resources"

install -m 755 "$binary" "$contents/MacOS/android-cli-gui"
sed "s/__VERSION__/$version/g" "$script_dir/Info.plist" > "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"

iconset="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$iconset"
trap 'rm -rf "$(dirname "$iconset")"' EXIT
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$script_dir/AppIcon.png" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$script_dir/AppIcon.png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/AppIcon.icns"

plutil -lint "$contents/Info.plist" >/dev/null

# Ad-hoc signing keeps the bundle self-consistent on Apple Silicon. Homebrew
# downloads do not carry a browser quarantine flag, so no notarization is
# required for cask installs.
codesign --force --deep --sign - "$app" >/dev/null
codesign --verify --deep --strict "$app"

echo "Built $app"
