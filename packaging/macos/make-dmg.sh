#!/bin/bash
# Build a drag-to-Applications disk image around a signed .app bundle.
#
# Usage: make-dmg.sh <app> <version> <output.dmg>
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <app> <version> <output.dmg>" >&2
  exit 2
fi

app=$1
version=$2
output=$3

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "make-dmg.sh requires macOS (hdiutil)." >&2
  exit 1
fi
if [[ ! -d "$app" ]]; then
  echo "App bundle not found: $app" >&2
  exit 1
fi
if [[ "$output" != *.dmg ]]; then
  echo "Output must end in .dmg: $output" >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$staging/root"
ditto "$app" "$staging/root/Android CLI GUI.app"
ln -s /Applications "$staging/root/Applications"

mkdir -p "$(dirname "$output")"
output_dir="$(cd "$(dirname "$output")" && pwd)"
output="$output_dir/$(basename "$output")"
rm -f "$output"
hdiutil create \
  -volname "Android CLI GUI" \
  -srcfolder "$staging/root" \
  -ov \
  -format UDZO \
  "$output" >/dev/null
hdiutil verify "$output" >/dev/null

echo "Built $output (Android CLI GUI $version)"
