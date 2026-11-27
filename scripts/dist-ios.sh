#!/usr/bin/env bash
# Release build of the iOS app, unsigned, into dist/bragi-VER-ios-unsigned.ipa: for sideloading with a
# tool that signs it with your own Apple ID (AltStore, SideStore, Sideloadly). App Store, TestFlight
# and developer-signed ipas need WeirdBrains' Apple Developer account and are not built here.
# The Rust bridge is built by app/hook/build.dart with the on-device model's engine (llama.cpp with Metal)
# and with build paths remapped; the hook runs cargo with a trimmed environment, so RUSTFLAGS set here
# would not reach it.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(sed -n 's/^version: *\([0-9][^+ ]*\).*/\1/p' app/pubspec.yaml)
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.0}"
( cd app && flutter build ios --release --no-codesign )
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/Payload" dist
cp -R app/build/ios/iphoneos/Runner.app "$stage/Payload/"
rm -f "dist/bragi-$ver-ios-unsigned.ipa"
( cd "$stage" && zip -qry "$root/dist/bragi-$ver-ios-unsigned.ipa" Payload )
ls -l dist/
