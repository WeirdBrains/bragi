#!/usr/bin/env bash
# Release build of the iOS app, unsigned, into dist/bragi-VER-ios-unsigned.ipa: for sideloading with a
# tool that signs it with your own Apple ID (AltStore, SideStore, Sideloadly). App Store, TestFlight
# and developer-signed ipas need WeirdBrains' Apple Developer account and are not built here.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(sed -n 's/^version: *\([0-9][^+ ]*\).*/\1/p' app/pubspec.yaml)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/Users/user"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.0}"
( cd app && flutter build ios --release --no-codesign )
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/Payload" dist
cp -R app/build/ios/iphoneos/Runner.app "$stage/Payload/"
rm -f "dist/bragi-$ver-ios-unsigned.ipa"
( cd "$stage" && zip -qry "$root/dist/bragi-$ver-ios-unsigned.ipa" Payload )
ls -l dist/
