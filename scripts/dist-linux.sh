#!/usr/bin/env bash
# Release build of the Linux desktop shell and every Linux package, into dist/:
#   bragi-VER-ARCH-unknown-linux-gnu.tar.gz and .zip (bin/, share/, LICENSE, README.md)
#   deb, rpm and Arch pkg.tar.zst, each as `bragi` (desktop entry and icons) and `bragi-standalone`
#   (the binary only), made with nfpm (packaging/nfpm/).
# Needs nfpm on PATH for the packages (the Release workflow installs a pinned copy); without it only the
# archives are made. Build paths are remapped so no build machine path ends up in the binary.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="bragi-desktop"))')
arch=$(uname -m)
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/home/user"
cargo build --release --locked -p bragi-desktop

name="bragi-$ver-$arch-unknown-linux-gnu"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
b="$stage/$name"
install -Dm755 "$target_dir/release/bragi" "$b/bin/bragi"
install -Dm644 packaging/linux/bragi.desktop "$b/share/applications/bragi.desktop"
mkdir -p "$b/share/icons"
cp -r brand/png/hicolor "$b/share/icons/"
install -Dm644 LICENSE "$b/LICENSE"
install -Dm644 README.md "$b/README.md"
mkdir -p dist
rm -f "dist/$name.tar.gz" "dist/$name.zip"
tar -C "$stage" --owner=0 --group=0 --numeric-owner -czf "dist/$name.tar.gz" "$name"
( cd "$stage" && zip -qr "$root/dist/$name.zip" "$name" )

if command -v nfpm >/dev/null; then
  export BRAGI_VERSION="$ver" BRAGI_BUNDLE="$b"
  case "$arch" in x86_64) export BRAGI_ARCH=amd64 ;; aarch64) export BRAGI_ARCH=arm64 ;; *) echo "unknown arch $arch" >&2; exit 1 ;; esac
  for cfg in bragi bragi-standalone; do
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
