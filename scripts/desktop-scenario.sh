#!/usr/bin/env bash
# Plays the shared scenario in the desktop shell inside a virtual X server, never on the
# live desktop: a clean environment (env -i), a private XDG_RUNTIME_DIR, no inherited
# DISPLAY or WAYLAND_DISPLAY, the demo profile in a temp dir. Screenshots land in $1.
#
#   scripts/desktop-scenario.sh OUT_DIR [path/to/bragi]
set -euo pipefail
out=$(realpath -m "${1:?usage: desktop-scenario.sh OUT_DIR [bragi]}")
bin=$(realpath "${2:-${CARGO_TARGET_DIR:-target}/debug/bragi}")
mkdir -p "$out"
tmp=${TMPDIR:-/tmp}
scratch=$(mktemp -d "$tmp/bragi-virtual.XXXXXX")
mkdir -m 0700 "$scratch/run" "$scratch/home"
trap 'rm -rf "$scratch"' EXIT

cat > "$scratch/inner.sh" <<'INNER'
#!/usr/bin/env bash
set -uo pipefail
out=$1 bin=$2
[ -z "${WAYLAND_DISPLAY:-}" ] || { echo "refusing: WAYLAND_DISPLAY is set" >&2; exit 3; }
case "${DISPLAY:-}" in :0|:1|"") echo "refusing: DISPLAY=${DISPLAY:-unset} is not a virtual server" >&2; exit 3;; esac
coproc APP { exec "$bin" --scenario 2>&1; }
pid=$APP_PID
sleep 0.3
if tr '\0' '\n' < "/proc/$pid/environ" | grep -q '^WAYLAND_DISPLAY='; then
  echo "refusing: the app inherited WAYLAND_DISPLAY" >&2; kill "$pid"; exit 3
fi
echo "app pid $pid on $(tr '\0' '\n' < "/proc/$pid/environ" | grep '^DISPLAY=')"
while IFS= read -r line <&"${APP[0]}"; do
  echo "$line"
  case "$line" in
    "bragi-scenario: screenshot "*) sleep 0.4; import -window root "$out/desktop-${line#bragi-scenario: screenshot }.png" ;;
  esac
done
wait "$pid"
INNER
chmod +x "$scratch/inner.sh"

env -i PATH=/usr/bin:/bin HOME="$scratch/home" TMPDIR="$tmp" XDG_RUNTIME_DIR="$scratch/run" LANG=C.UTF-8 \
  BRAGI_SCENARIO_PAUSE_MS="${BRAGI_SCENARIO_PAUSE_MS:-900}" RUST_LOG=warn BRAGI_AUDIO=null \
  xvfb-run -a -s "-screen 0 1100x720x24 -nolisten tcp" "$scratch/inner.sh" "$out" "$bin"
