#!/usr/bin/env bash
# Boots a throwaway Android emulator with no window (-no-window, env -i, a private
# XDG_RUNTIME_DIR, AVD home and adb server port, no DISPLAY or WAYLAND_DISPLAY), installs a demo
# APK (built with --dart-define=BRAGI_DEMO=true), and saves screenshots in OUT_DIR as the
# demo links and chats. The taps are driven with `adb shell input`.
#
#   scripts/android-scenario.sh APK AVD_HOME AVD_NAME OUT_DIR
set -euo pipefail
apk=$(realpath "$1") avd_home=$(realpath "$2") avd=$3 out=$(realpath -m "$4")
sdk=${ANDROID_HOME:-$HOME/Android/Sdk}
mkdir -p "$out"
scratch=$(mktemp -d /tmp/bragi-android.XXXXXX)
mkdir -m 0700 "$scratch/run"
# adb keys stay with the AVD (a fresh key each run is refused by Play images).
mkdir -p -m 0700 "$avd_home/home"
port=${BRAGI_ADB_PORT:-5199}
clean() {
  ANDROID_ADB_SERVER_PORT=$port "$sdk/platform-tools/adb" -s emulator-5682 emu kill >/dev/null 2>&1 || true
  sleep 2
  [ -n "${emu_pid:-}" ] && kill "$emu_pid" 2>/dev/null || true
  ANDROID_ADB_SERVER_PORT=$port "$sdk/platform-tools/adb" kill-server >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap clean EXIT
E=(env -i PATH=/usr/bin:/bin HOME="$avd_home/home" XDG_RUNTIME_DIR="$scratch/run" ANDROID_HOME="$sdk" ANDROID_SDK_ROOT="$sdk"
   ANDROID_AVD_HOME="$avd_home" ANDROID_ADB_SERVER_PORT="$port")
adb() { "${E[@]}" "$sdk/platform-tools/adb" -s emulator-5682 "$@"; }

"${E[@]}" "$sdk/emulator/emulator" -avd "$avd" -port 5682 -no-window -no-audio -no-snapshot -wipe-data -no-boot-anim \
  -gpu swiftshader_indirect -memory 3072 -cores 2 >"$scratch/emulator.log" 2>&1 &
emu_pid=$!
sleep 3
env_of=$(tr '\0' '\n' < "/proc/$emu_pid/environ")
if grep -qE '^(DISPLAY|WAYLAND_DISPLAY)=' <<<"$env_of"; then echo "refusing: display inherited" >&2; exit 3; fi
echo "emulator pid $emu_pid, no DISPLAY/WAYLAND_DISPLAY in its environment"
"${E[@]}" "$sdk/platform-tools/adb" start-server >/dev/null
for _ in $(seq 1 240); do
  [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && break
  sleep 2
done
adb shell getprop sys.boot_completed | grep -q 1 || { tail -20 "$scratch/emulator.log"; exit 1; }
echo booted
# The package manager comes up a little after boot_completed on a wiped device.
for i in $(seq 1 30); do adb install -r "$apk" >/dev/null 2>&1 && break; [ "$i" = 30 ] && { echo "install failed" >&2; exit 1; }; sleep 4; done
echo installed
adb shell settings put global window_animation_scale 0
adb shell settings put global transition_animation_scale 0
adb shell monkey -p org.weirdbrains.bragi -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
shot() { sleep "${2:-1}"; adb exec-out screencap -p > "$out/android-$1.png"; echo "screenshot $1"; }
# UI coordinates come from uiautomator, by visible text.
tap_text() {
  for _ in $(seq 1 60); do
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
    xy=$(adb shell cat /sdcard/ui.xml 2>/dev/null | python3 -c "
import re,sys
x=sys.stdin.read()
for m in re.finditer(r'<node [^>]*>', x):
    n=m.group(0)
    if ('text=\"$1\"' in n) or ('content-desc=\"$1\"' in n):
        b=re.search(r'bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"', n)
        if b:
            a=list(map(int,b.groups())); print((a[0]+a[2])//2,(a[1]+a[3])//2); break
")
    if [ -n "$xy" ]; then LAST_XY=$xy; adb shell input tap $xy; return 0; fi
    sleep 1
  done
  echo "not found: $1" >&2; return 1
}
wait_text() {
  for _ in $(seq 1 90); do
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
    adb shell cat /sdcard/ui.xml 2>/dev/null | grep -q "$1" && return 0
    sleep 1
  done
  echo "never showed: $1" >&2; return 1
}
tap_field() {
  adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
  xy=$(adb shell cat /sdcard/ui.xml | python3 -c "
import re,sys
for n in re.findall(r'<node [^>]*class=\"android.widget.EditText\"[^>]*>', sys.stdin.read()):
    a=list(map(int,re.search(r'bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"', n).groups())); print((a[0]+a[2])//2,(a[1]+a[3])//2); break
")
  adb shell input tap $xy
}
type_prompt() {
  tap_field
  adb shell input text "$(sed 's/ /%s/g' <<<"$1")"
  # Hide the keyboard (and its autocorrect) before the Send tap and the screenshots.
  adb shell input keyevent 111
}
wait_text "Show a link code"; shot 01-unpaired
tap_text "Show a link code"; wait_text "Open this code\|They match"; shot 02-code 0
wait_text "They match"; shot 03-check 0.3
tap_text "They match"
wait_text "gemma-4-e4b"; shot 04-models 1.5
type_prompt "What does Bragi do?"; tap_text "Send"
wait_text "Continue your\|a cut-off answer\|another one\." ; shot 05-answer 1
type_prompt "Tell me more about cancel."; tap_text "Send"
# Stop sits where Send was; a uiautomator dump is too slow to find it mid-answer.
sleep 1; adb shell input tap $LAST_XY; shot 06-stopping 0; wait_text "stopped"; shot 07-stopped 0.3
tap_text "Continue"; sleep 6; shot 08-continued 0
adb shell uiautomator dump /sdcard/ui.xml >/dev/null
n=$(adb shell cat /sdcard/ui.xml | grep -o 'another one\.' | wc -l)
[ "$n" -ge 2 ] && echo "android-scenario: passed (both answers complete)" || { echo "android-scenario: FAILED ($n complete answers)"; exit 1; }
