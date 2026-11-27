#!/usr/bin/env bash
# Release build of the Linux desktop shell, the terminal app and every Linux package, into dist/:
#   bragi-VER-ARCH-unknown-linux-gnu.tar.gz and .zip (bin/bragi, bin/bragi-tui -> bragi, share/, LICENSE, README.md)
#   bragi-tui-VER-ARCH-unknown-linux-gnu.tar.gz (the terminal app alone, for servers and SSH)
#   deb, rpm and Arch pkg.tar.zst, each as `bragi` (desktop entry and icons), `bragi-standalone`
#   (the binaries only) and `bragi-tui` (the terminal app only, no GUI libraries), made with nfpm
#   (packaging/nfpm/).
# The archives bundle the LGPL ffmpeg and ffprobe (bin/, beside bragi) when BRAGI_FFMPEG_DIR, or
# dist/ffmpeg/<host triple>, holds a build from scripts/build-ffmpeg.sh; without one they go out
# without it and video attach asks for the distro's ffmpeg. The deb, rpm and Arch packages never
# bundle it: they recommend the distro's ffmpeg instead.
# Cargo features: BRAGI_FEATURES (default `on-device`, the optional on-device model's llama.cpp engine, B4;
# portable: llama.cpp is built without -march=native) goes into both builds; BRAGI_FEATURES= builds without.
# The engine links libstdc++, which every package depends on.
# Needs nfpm on PATH for the packages (the Release workflow installs a pinned copy); without it only the
# archives are made. Build paths are remapped so no build machine path ends up in the binary.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="bragi-desktop"))')
arch=$(uname -m)
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/home/user"
features="${BRAGI_FEATURES-on-device}"
feature_args=()
[[ -n "$features" ]] && feature_args=(--features "$features")
echo "features: ${features:-none}"
cargo build --release --locked -p bragi-desktop "${feature_args[@]}"

name="bragi-$ver-$arch-unknown-linux-gnu"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
b="$stage/$name"
install -Dm755 "$target_dir/release/bragi" "$b/bin/bragi"
ln -s bragi "$b/bin/bragi-tui"
install -Dm644 packaging/linux/bragi.desktop "$b/share/applications/bragi.desktop"
mkdir -p "$b/share/icons"
cp -r brand/png/hicolor "$b/share/icons/"
install -Dm644 LICENSE "$b/LICENSE"
install -Dm644 README.md "$b/README.md"
install -Dm644 THIRD_PARTY_NOTICES.md "$b/THIRD_PARTY_NOTICES.md"
host=$(rustc -vV | sed -n 's/^host: //p')
ffdir="${BRAGI_FFMPEG_DIR:-dist/ffmpeg/$host}"
# Copies the bundled ffmpeg, its licence, build config and source offer into the archive at $1.
bundle_ffmpeg() {
  if [[ -x "$ffdir/bin/ffmpeg" && -x "$ffdir/bin/ffprobe" ]]; then
    install -Dm755 "$ffdir/bin/ffmpeg" "$1/bin/ffmpeg"
    install -Dm755 "$ffdir/bin/ffprobe" "$1/bin/ffprobe"
    for f in LICENSE.md CONFIG.txt SOURCE.txt; do install -Dm644 "$ffdir/$f" "$1/share/doc/ffmpeg/$f"; done
  else
    echo "no ffmpeg build in $ffdir (scripts/build-ffmpeg.sh): $(basename "$1") goes out without one" >&2
  fi
}
bundle_ffmpeg "$b"
mkdir -p dist
rm -f "dist/$name.tar.gz" "dist/$name.zip"
tar -C "$stage" --owner=0 --group=0 --numeric-owner -czf "dist/$name.tar.gz" "$name"
( cd "$stage" && zip -qr "$root/dist/$name.zip" "$name" )
tui="bragi-tui-$ver-$arch-unknown-linux-gnu"
# The terminal-only build: the same `bragi` without the `gui` feature, so no GUI libraries.
cargo build --release --locked -p bragi-desktop --no-default-features "${feature_args[@]}"
install -Dm755 "$target_dir/release/bragi" "$stage/$tui/bin/bragi"
ln -s bragi "$stage/$tui/bin/bragi-tui"
install -Dm644 LICENSE "$stage/$tui/LICENSE"
install -Dm644 README.md "$stage/$tui/README.md"
install -Dm644 THIRD_PARTY_NOTICES.md "$stage/$tui/THIRD_PARTY_NOTICES.md"
bundle_ffmpeg "$stage/$tui"
rm -f "dist/$tui.tar.gz"
tar -C "$stage" --owner=0 --group=0 --numeric-owner -czf "dist/$tui.tar.gz" "$tui"

if command -v nfpm >/dev/null; then
  export BRAGI_VERSION="$ver" BRAGI_BUNDLE="$b"
  case "$arch" in x86_64) export BRAGI_ARCH=amd64 ;; aarch64) export BRAGI_ARCH=arm64 ;; *) echo "unknown arch $arch" >&2; exit 1 ;; esac
  for cfg in bragi bragi-standalone bragi-tui; do
    # Expand ${BRAGI_*} here so nothing depends on which fields nfpm expands itself.
    python3 -c 'import os,sys;print(os.path.expandvars(open(sys.argv[1]).read()))' "packaging/nfpm/$cfg.yaml" > "$stage/$cfg.yaml"
    for p in deb rpm archlinux; do
      nfpm package --config "$stage/$cfg.yaml" --packager "$p" --target dist/
    done
  done
else
  echo "nfpm not found: archives only (no deb, rpm or pkg.tar.zst)" >&2
fi
ls -l dist/
