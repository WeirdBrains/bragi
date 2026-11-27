#!/usr/bin/env bash
# Release build of the macOS desktop shell into dist/: a universal (arm64 + x86_64) Bragi.app in
# bragi-VER-universal-apple-darwin.zip and .dmg, and the bare universal binaries (bragi and the
# terminal app: `bragi` runs it, `bragi desktop` opens the window; bragi-tui is a symlink to bragi) in .tar.gz.
# The app is ad-hoc signed (codesign -s -), not Developer ID signed or notarized: Gatekeeper asks
# the first time (right-click, Open). Build paths are remapped out of the binary.
# With BRAGI_FFMPEG_DIR (universal ffmpeg and ffprobe from scripts/build-ffmpeg.sh, built per arch with
# FFMPEG_CROSS and joined with lipo), they go in Bragi.app/Contents/Helpers, signed with the app, and
# beside bragi in the tarball; Bragi finds them there before PATH.
# BRAGI_FEATURES (default none) adds Cargo features, e.g. `on-device` (the llama.cpp engine; not turned on
# in the Release workflow for macOS yet).
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="bragi-desktop"))')
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/Users/user"
features="${BRAGI_FEATURES:-}"
feature_args=()
[[ -n "$features" ]] && feature_args=(--features "$features")
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked -p bragi-desktop --target "$t" ${feature_args[@]+"${feature_args[@]}"}
done
name="bragi-$ver-universal-apple-darwin"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
lipo -create -output "$stage/bragi" "$target_dir/aarch64-apple-darwin/release/bragi" "$target_dir/x86_64-apple-darwin/release/bragi"
lipo -info "$stage/bragi"
app="$stage/app/Bragi.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$stage/bragi" "$app/Contents/MacOS/bragi"
# Opening the app runs `bragi desktop`; `bragi` alone is the terminal app.
cat > "$app/Contents/MacOS/bragi-app" <<'LAUNCH'
#!/bin/sh
exec "$(dirname "$0")/bragi" desktop "$@"
LAUNCH
chmod 755 "$app/Contents/MacOS/bragi-app"
cp brand/png/macos/bragi.icns "$app/Contents/Resources/bragi.icns"
cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
ffdir="${BRAGI_FFMPEG_DIR:-}"
if [[ -n "$ffdir" ]]; then
  mkdir -p "$app/Contents/Helpers" "$app/Contents/Resources/ffmpeg"
  cp "$ffdir/bin/ffmpeg" "$ffdir/bin/ffprobe" "$app/Contents/Helpers/"
  cp "$ffdir/LICENSE.md" "$ffdir/CONFIG.txt" "$ffdir/SOURCE.txt" "$app/Contents/Resources/ffmpeg/"
  # Helpers are signed first, like the binary: an unsigned helper breaks notarisation.
  codesign --force --sign - --timestamp=none "$app/Contents/Helpers/ffmpeg" "$app/Contents/Helpers/ffprobe"
fi
sed "s/@VERSION@/$ver/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
# The binary first: it is not the bundle's main executable (bragi-app is), so signing the app alone leaves it unsigned.
codesign --force --sign - --timestamp=none "$app/Contents/MacOS/bragi"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --verbose "$app"
mkdir -p dist
rm -f "dist/$name".*
( cd "$stage/app" && ditto -c -k --keepParent Bragi.app "$root/dist/$name.zip" )
ln -s /Applications "$stage/app/Applications"
# hdiutil's own size guess for -srcfolder can come up short ("No space left on device"), so the image
# is sized from the folder with room to spare, and a failed create is tried again.
dmg_mb=$(( $(du -sm "$stage/app" | cut -f1) * 12 / 10 + 32 ))
for attempt in 1 2 3; do
  hdiutil create -volname "Bragi $ver" -srcfolder "$stage/app" -size "${dmg_mb}m" -ov -format UDZO "dist/$name.dmg" && break
  [[ $attempt == 3 ]] && exit 1
  echo "hdiutil create failed (attempt $attempt), trying again" >&2
  sleep 5
done
mkdir -p "$stage/tar/$name"
cp "$stage/bragi" "$stage/tar/$name/bragi"
ln -s bragi "$stage/tar/$name/bragi-tui"
cp LICENSE README.md THIRD_PARTY_NOTICES.md "$stage/tar/$name/"
if [[ -n "$ffdir" ]]; then
  cp "$ffdir/bin/ffmpeg" "$ffdir/bin/ffprobe" "$stage/tar/$name/"
  mkdir -p "$stage/tar/$name/ffmpeg" && cp "$ffdir/LICENSE.md" "$ffdir/CONFIG.txt" "$ffdir/SOURCE.txt" "$stage/tar/$name/ffmpeg/"
fi
tar -C "$stage/tar" -czf "dist/$name.tar.gz" "$name"
ls -l dist/
