#!/usr/bin/env bash
# Builds the Android release artefacts into dist/android: one apk per ABI (for F-Droid and
# sideloading) and the aab (for Play). Signs with app/android/key.properties when it exists, else
# with the debug key (local testing only: the output says which).
set -euo pipefail
cd "$(dirname "$0")/../app"
out=../dist/android
mkdir -p "$out"
if [ -f android/key.properties ]; then echo "signing: release key"; else echo "signing: DEBUG key (not for upload)"; fi
flutter build apk --release --split-per-abi
flutter build appbundle --release
cp build/app/outputs/flutter-apk/app-*-release.apk "$out/"
cp build/app/outputs/bundle/release/app-release.aab "$out/"
( cd "$out" && sha256sum * > SHA256SUMS )
ls -l "$out"
