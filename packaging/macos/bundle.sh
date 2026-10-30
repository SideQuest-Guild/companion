#!/usr/bin/env bash
# Assembles SideQuest Companion.app from one or two built binaries. Used by both `make bundle-macos` (one binary, the
# host arch, lipo is a no-op) and the release workflow (arm64 + x86_64 binaries combined into a universal binary).
#
# Usage: bundle.sh <version> <arm64-binary> <x86_64-binary> <out-dir>
# Writes <out-dir>/SideQuest Companion.app

set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <version> <arm64-binary> <x86_64-binary> <out-dir>" >&2
  exit 1
fi

version="$1"
arm64_bin="$2"
x86_64_bin="$3"
out_dir="$4"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"

app="$out_dir/SideQuest Companion.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

if [ "$arm64_bin" = "$x86_64_bin" ]; then
  # Local single-arch build: no universal binary to make.
  cp "$arm64_bin" "$app/Contents/MacOS/sidequest-companion"
else
  lipo -create -output "$app/Contents/MacOS/sidequest-companion" "$arm64_bin" "$x86_64_bin"
fi
chmod +x "$app/Contents/MacOS/sidequest-companion"

sed "s/@VERSION@/$version/g" "$script_dir/Info.plist" > "$app/Contents/Info.plist"

if [ -f "$repo_root/assets/icon.icns" ]; then
  cp "$repo_root/assets/icon.icns" "$app/Contents/Resources/icon.icns"
else
  echo "warning: assets/icon.icns not found, bundling without an app icon" >&2
fi

# Ad-hoc signature: unsigned binaries are blocked outright on Apple Silicon, and this is the same signing CI does.
# Users still get a Gatekeeper "unidentified developer" prompt on first launch; see README.md.
codesign --force --deep -s - "$app"

echo "$app"
