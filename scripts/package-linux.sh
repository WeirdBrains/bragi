#!/usr/bin/env bash
# Builds the Linux desktop shell and packs dist/bragi-VERSION-linux-ARCH.tar.gz with the binary,
# the .desktop file and the hicolor icons, laid out as a /usr prefix.
set -euo pipefail
cd "$(dirname "$0")/.."
ver=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="bragi-desktop"))')
arch=$(uname -m)
cargo build --release -p bragi-desktop
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root="$stage/bragi-$ver"
install -Dm755 "$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/bragi" "$root/usr/bin/bragi"
install -Dm644 packaging/linux/bragi.desktop "$root/usr/share/applications/bragi.desktop"
cp -r brand/png/hicolor "$root/usr/share/icons/" 2>/dev/null || { mkdir -p "$root/usr/share/icons"; cp -r brand/png/hicolor "$root/usr/share/icons/"; }
mkdir -p dist
tar -C "$stage" -czf "dist/bragi-$ver-linux-$arch.tar.gz" "bragi-$ver"
sha256sum "dist/bragi-$ver-linux-$arch.tar.gz"
