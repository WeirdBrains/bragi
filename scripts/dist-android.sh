#!/usr/bin/env bash
# Release build of the Android app into dist/: one apk per ABI (arm64-v8a, armeabi-v7a, x86_64) for
# sideloading and F-Droid-style installs, and the aab for Google Play.
#
# Signing: with BRAGI_ANDROID_KEYSTORE (a keystore file), BRAGI_ANDROID_KEYSTORE_PASSWORD,
# BRAGI_ANDROID_KEY_ALIAS and BRAGI_ANDROID_KEY_PASSWORD set, the release key signs and the files are
# bragi-VER-android-ABI.apk and bragi-VER-android.aab. Without them Flutter signs with the debug key:
# the apks are named bragi-VER-android-ABI-debug-signed.apk (sideload only, they will not update an
# install signed by another key) and no aab is made (Play refuses debug-signed bundles).
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
ver=$(sed -n 's/^version: *\([0-9][^+ ]*\).*/\1/p' app/pubspec.yaml)
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$root=/build/bragi --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$HOME=/home/user"
props=app/android/key.properties
cleanup() { [ -n "${made_props:-}" ] && rm -f "$props"; }
trap cleanup EXIT
if [ -n "${BRAGI_ANDROID_KEYSTORE:-}" ]; then
  [ -f "$props" ] && { echo "$props exists; not overwriting it" >&2; exit 1; }
  made_props=1
  umask 077
  {
    echo "storeFile=$BRAGI_ANDROID_KEYSTORE"
    echo "storePassword=$BRAGI_ANDROID_KEYSTORE_PASSWORD"
    echo "keyAlias=$BRAGI_ANDROID_KEY_ALIAS"
    echo "keyPassword=$BRAGI_ANDROID_KEY_PASSWORD"
  } > "$props"
  suffix=""
  echo "signing: release key"
else
  suffix="-debug-signed"
  echo "signing: DEBUG key (sideload only; no aab)"
fi
cd app
flutter build apk --release --split-per-abi
mkdir -p ../dist
for abi in arm64-v8a armeabi-v7a x86_64; do
  cp "build/app/outputs/flutter-apk/app-$abi-release.apk" "../dist/bragi-$ver-android-$abi$suffix.apk"
done
if [ -z "$suffix" ]; then
  flutter build appbundle --release
  cp build/app/outputs/bundle/release/app-release.aab "../dist/bragi-$ver-android.aab"
fi
ls -l ../dist/
