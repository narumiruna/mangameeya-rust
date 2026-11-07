#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
if [[ "$(uname -s)" != Darwin || $# -gt 1 ]]; then
    printf 'Usage on macOS: bash packaging/macos/build.sh [aarch64-apple-darwin|x86_64-apple-darwin]\n' >&2
    exit 1
fi
export DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}"
if [[ ! -d "$DEVELOPER_DIR/Platforms/MacOSX.platform" ]]; then
    printf 'Full Xcode is required. Set DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer.\n' >&2
    exit 1
fi
xcrun --sdk macosx --find metal >/dev/null
export MACOSX_DEPLOYMENT_TARGET=12.0

target="${1:-$(rustc -vV | awk '$1 == "host:" { print $2 }')}"
case "$target" in
    aarch64-apple-darwin) architecture=arm64 ;;
    x86_64-apple-darwin) architecture=x86_64 ;;
    *) printf 'Unsupported macOS target: %s\n' "$target" >&2; exit 1 ;;
esac
package_id="$(cargo pkgid --locked --package mangameeya-rust)"
version="${package_id##*#}"
version="${version##*@}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
    printf 'Cannot read the Cargo package version.\n' >&2
    exit 1
fi
numeric_version="${version%%[-+]*}"
cargo build --locked --release --bin mangameeya-rust --target "$target" --target-dir "$root/target"

executable="$root/target/$target/release/mangameeya-rust"
bundle_dir="$root/target/release/bundle/macos/$target"
app="$bundle_dir/MangaMeeya Rust.app"
mkdir -p "$bundle_dir"
work_dir="$(mktemp -d "$bundle_dir/.macos-packaging.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$executable" "$app/Contents/MacOS/mangameeya-rust"
chmod 755 "$app/Contents/MacOS/mangameeya-rust"
cp packaging/macos/Info.plist "$app/Contents/Info.plist"
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
printf 'APPL????' > "$app/Contents/PkgInfo"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $numeric_version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $numeric_version" "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
lipo "$app/Contents/MacOS/mangameeya-rust" -verify_arch "$architecture"

# Generate the native icon from the original application PNG, with no extra tools.
iconset="$work_dir/icon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/icon-256.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" assets/icon-256.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/icon.icns"

# Ad-hoc signing is local integrity metadata, not Developer ID or notarization.
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict "$app"
zip="$bundle_dir/MangaMeeya-Rust_${version}_${target}.zip"
dmg="$bundle_dir/MangaMeeya-Rust_${version}_${target}.dmg"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
mkdir -p "$work_dir/dmg"
ditto "$app" "$work_dir/dmg/MangaMeeya Rust.app"
ln -s /Applications "$work_dir/dmg/Applications"
hdiutil create -quiet -volname 'MangaMeeya Rust' -srcfolder "$work_dir/dmg" -ov -format UDZO "$dmg" </dev/null
hdiutil verify "$dmg" </dev/null
printf 'Created %s\nCreated %s\n' "$zip" "$dmg"
