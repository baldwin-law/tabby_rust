#!/usr/bin/env bash
# Build a macOS .app bundle plus zip and dmg.
#
# Usage: package.sh <binary> <version> <arch> <output-dir>
#
# Set APPLE_SIGNING_IDENTITY for a Developer ID signature. When APPLE_ID,
# APPLE_TEAM_ID and APPLE_APP_PASSWORD are also set, the app is notarized and
# stapled. Without those secrets the bundle is ad-hoc signed, which is enough
# for local testing but downloaded copies still need the quarantine override.
set -euo pipefail

binary=${1:?binary path required}
version=${2:?version required}
arch=${3:?arch required}
out_dir=${4:?output directory required}

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$script_dir/../.." && pwd)
app_name="Tabby Rust"
app="$out_dir/$app_name.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 0755 "$binary" "$app/Contents/MacOS/tabby-rust"
sed "s/@VERSION@/$version/g" "$script_dir/Info.plist" > "$app/Contents/Info.plist"
install -m 0644 "$root/README.md" "$root/LICENSE" "$app/Contents/Resources/"

icons_dir="$script_dir/../icons"
if command -v iconutil >/dev/null 2>&1; then
  iconset="$out_dir/tabby-rust.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"
  cp "$icons_dir/tabby-rust-16.png"  "$iconset/icon_16x16.png"
  cp "$icons_dir/tabby-rust-32.png"  "$iconset/icon_16x16@2x.png"
  cp "$icons_dir/tabby-rust-32.png"  "$iconset/icon_32x32.png"
  cp "$icons_dir/tabby-rust-64.png"  "$iconset/icon_32x32@2x.png"
  cp "$icons_dir/tabby-rust-128.png" "$iconset/icon_128x128.png"
  cp "$icons_dir/tabby-rust-256.png" "$iconset/icon_128x128@2x.png"
  cp "$icons_dir/tabby-rust-256.png" "$iconset/icon_256x256.png"
  cp "$icons_dir/tabby-rust-512.png" "$iconset/icon_256x256@2x.png"
  cp "$icons_dir/tabby-rust-512.png" "$iconset/icon_512x512.png"
  cp "$icons_dir/tabby-rust-1024.png" "$iconset/icon_512x512@2x.png"
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/tabby-rust.icns"
  rm -rf "$iconset"
else
  echo "iconutil not found; shipping a generic app icon" >&2
fi

plutil -lint "$app/Contents/Info.plist" >/dev/null

if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  codesign --force --deep --options runtime --timestamp \
    --sign "$APPLE_SIGNING_IDENTITY" "$app"
else
  # Ad-hoc signing keeps modern macOS from rejecting the unsigned bundle.
  codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true
fi

if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  notarize_zip="$out_dir/notarize-$arch.zip"
  (cd "$out_dir" && ditto -c -k --sequesterRsrc --keepParent "$app_name.app" "$notarize_zip")
  xcrun notarytool submit "$notarize_zip" \
    --apple-id "$APPLE_ID" \
    --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --wait
  xcrun stapler staple "$app"
  rm -f "$notarize_zip"
else
  echo "Apple signing/notarization secrets not set; shipping an ad-hoc signed app" >&2
fi

zip_path="$out_dir/tabby-rust-$version-macos-$arch.zip"
rm -f "$zip_path"
(cd "$out_dir" && ditto -c -k --sequesterRsrc --keepParent "$app_name.app" "$zip_path")
echo "wrote $zip_path"

dmg_path="$out_dir/tabby-rust-$version-macos-$arch.dmg"
rm -f "$dmg_path"
hdiutil create -volname "$app_name" -srcfolder "$app" -ov -format UDZO "$dmg_path" >/dev/null
echo "wrote $dmg_path"
