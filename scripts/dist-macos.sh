#!/usr/bin/env bash
# Release build of the macOS desktop shell into dist/: a universal (arm64 + x86_64) Bragi.app in
# bragi-VER-universal-apple-darwin.zip and .dmg, and the bare universal binaries (bragi and the
# terminal app bragi-tui) in .tar.gz. The app also carries bragi-tui in Contents/MacOS.
# The app is ad-hoc signed (codesign -s -), not Developer ID signed or notarized: Gatekeeper asks
# the first time (right-click, Open). Build paths are remapped out of the binary.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="bragi-desktop"))')
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/Users/user"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked -p bragi-desktop -p bragi-tui --target "$t"
done
name="bragi-$ver-universal-apple-darwin"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
lipo -create -output "$stage/bragi" "$target_dir/aarch64-apple-darwin/release/bragi" "$target_dir/x86_64-apple-darwin/release/bragi"
lipo -create -output "$stage/bragi-tui" "$target_dir/aarch64-apple-darwin/release/bragi-tui" "$target_dir/x86_64-apple-darwin/release/bragi-tui"
lipo -info "$stage/bragi" "$stage/bragi-tui"
app="$stage/app/Bragi.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$stage/bragi" "$app/Contents/MacOS/bragi"
cp "$stage/bragi-tui" "$app/Contents/MacOS/bragi-tui"
cp brand/png/macos/bragi.icns "$app/Contents/Resources/bragi.icns"
sed "s/@VERSION@/$ver/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --verbose "$app"
mkdir -p dist
rm -f "dist/$name".*
( cd "$stage/app" && ditto -c -k --keepParent Bragi.app "$root/dist/$name.zip" )
ln -s /Applications "$stage/app/Applications"
hdiutil create -volname "Bragi $ver" -srcfolder "$stage/app" -ov -format UDZO "dist/$name.dmg"
mkdir -p "$stage/tar/$name"
cp "$stage/bragi" "$stage/tar/$name/bragi"
cp "$stage/bragi-tui" "$stage/tar/$name/bragi-tui"
cp LICENSE README.md "$stage/tar/$name/"
tar -C "$stage/tar" -czf "dist/$name.tar.gz" "$name"
ls -l dist/
