#!/usr/bin/env bash
# End to end: a headless Bragi core and real `ymird` nodes link each other in both directions, with
# throwaway state and a throwaway identity, over 127.0.0.1 only (no relays, no discovery on the Ymir
# side). Then Bragi lists the pool's models and gets a chat reply from the testkit's mock engine.
#
#   YMIR_BIN=/path/to/ymir/target/debug scripts/ymir-link-e2e.sh
#
# YMIR_BIN holds `ymird`, `ymirctl` and `ymir-mock-engine` (cargo build -p ymird -p ymirctl -p
# ymir-testkit --bins in the Ymir repo). BRAGI_EXAMPLE is the built `ymir_link_e2e` example
# (cargo build -p bragi-core --example ymir_link_e2e). Nothing touches a live pool or opens a window.
set -euo pipefail

: "${YMIR_BIN:?set YMIR_BIN to the directory with ymird, ymirctl and ymir-mock-engine}"
: "${BRAGI_EXAMPLE:?set BRAGI_EXAMPLE to the built ymir_link_e2e example}"
ROOT=$(mktemp -d "${TMPDIR:-/tmp}/bragi-ymir-e2e.XXXXXX")
export XDG_RUNTIME_DIR="$ROOT/run"; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; echo "(state kept in $ROOT)" >&2; trap - EXIT; cleanup; exit 1; }
ok() { echo "ok: $*"; }

YMIRCTL() { local home=$1; shift; env -i PATH="$PATH" HOME="$ROOT/home" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" "$YMIR_BIN/ymirctl" --home "$home" "$@"; }

node_toml() { # home name roles(model|inference,app|...) peers
  local home=$1 name=$2 roles=$3 peers=${4:-}
  mkdir -p "$home" "$ROOT/models"; echo "GGUF mock" > "$ROOT/models/alpha-$name.gguf"
  local m=false i=false a=false local_model=""
  [[ $roles == *model* ]] && m=true && local_model="[[model.local]]
id = \"alpha-$name\"
file = \"alpha-$name.gguf\"
role = \"main\"
"
  [[ $roles == *inference* ]] && i=true
  [[ $roles == *app* ]] && a=true
  cat > "$home/node.toml" <<TOML
[node]
name = "$name"
[roles]
model = $m
inference = $i
app = $a
[network]
listen = ["/ip4/127.0.0.1/tcp/0"]
mdns = false
discovery = false
relays = []
sync_secs = 1
peers = [$peers]
[model]
llamacpp = "system:$YMIR_BIN/ymir-mock-engine"
vllm = "off"
models_dir = "$ROOT/models"
${local_model}[inference.alerts]
probe_secs = 3600
[app.bridge]
port = 0
TOML
}

start_ymird() { # home
  env -i PATH="$PATH" HOME="$ROOT/home" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" YMIR_HOME="$1" "$YMIR_BIN/ymird" > "$1/ymird.log" 2>&1 &
  PIDS+=($!)
}

node_addr() { # home -> /ip4/127.0.0.1/tcp/PORT/p2p/PEER
  local st; st=$(YMIRCTL "$1" status)
  local peer; peer=$(sed -n 's/^Weirdnet: .* as \([^,]*\),.*/\1/p' <<<"$st")
  local l; l=$(sed -n 's/^  listening \(\/ip4\/127\.0\.0\.1\/tcp\/[0-9]*\).*/\1/p' <<<"$st" | head -1)
  [[ -n $peer && -n $l ]] || return 1
  echo "$l/p2p/$peer"
}

wait_for() { # secs description command...
  local secs=$1 what=$2; shift 2; local end=$((SECONDS + secs))
  until "$@" >/dev/null 2>&1; do [[ $SECONDS -lt $end ]] || fail "timed out waiting for $what"; sleep 0.5; done
}

mkdir -p "$ROOT/home" "$ROOT/work"
WORK="$ROOT/work"

# 1. A Ymir node with a throwaway identity of its own (all three roles, mock engine).
N1="$ROOT/ymir1"; node_toml "$N1" node1 "model,inference,app"
YMIRCTL "$N1" pool init --standalone >/dev/null || fail "pool init"
start_ymird "$N1"
wait_for 60 "node1 on Weirdnet" node_addr "$N1"
ADDR1=$(node_addr "$N1"); ok "node1 is up on Weirdnet at a loopback address"
USER1=$(YMIRCTL "$N1" pool devices | sed -n 's/^Identity //p')

# 2. Bragi (headless) shows its code; `ymirctl pool add` opens it. Check the codes match.
env -i PATH="$PATH" HOME="$ROOT/home" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" RUST_LOG=warn BRAGI_E2E_CERT_ONLY="${BRAGI_E2E_CERT_ONLY:-}" \
  "$BRAGI_EXAMPLE" "$ROOT/bragi" "$ADDR1" "$WORK" > "$ROOT/bragi.out" 2>&1 &
BPID=$!; PIDS+=($BPID)
wait_for 60 "Bragi's link code" test -s "$WORK/bragi.link"
CODE=$(cat "$WORK/bragi.link")
[[ $CODE == weirdnet-bragi://link/* ]] || fail "Bragi's code has an unexpected scheme"
ok "Bragi shows a weirdnet-bragi:// code"
ADD=$(YMIRCTL "$N1" pool add "$CODE" --yes) || fail "ymirctl pool add refused Bragi's code"
echo "$ADD"
YSAS=$(sed -n 's/.*the code is \([0-9]*\).*/\1/p' <<<"$ADD")
wait_for 30 "Bragi's check code" test -s "$WORK/bragi.sas"
[[ "$YSAS" == "$(cat "$WORK/bragi.sas")" ]] || fail "check codes differ: ymirctl $YSAS, Bragi $(cat "$WORK/bragi.sas")"
ok "ymirctl pool add took Bragi's code; both showed the same check code $YSAS"
wait_for 120 "Bragi's first chat" test -f "$WORK/bragi.done1" || fail "Bragi did not finish its chat"
grep -q "^BRAGI_MODELS alpha-node1" "$ROOT/bragi.out" || { cat "$ROOT/bragi.out"; fail "Bragi did not list node1's model"; }
grep -q "^BRAGI_REPLY Hello from alpha-node1! You said: " "$ROOT/bragi.out" || { cat "$ROOT/bragi.out"; fail "Bragi got no chat reply"; }
ok "Bragi is linked (direction 1), lists the pool's models and got a reply: $(grep '^BRAGI_REPLY' "$ROOT/bragi.out")"
YMIRCTL "$N1" pool devices | grep -q "bragi-e2e" || fail "node1 roster does not list Bragi"
ok "node1 roster lists Bragi (bragi-e2e)"

# 3. A second Ymir node shows `ymirctl pool link`; the linked Bragi opens that code.
N2="$ROOT/ymir2"; node_toml "$N2" node2 "model" "\"$ADDR1\""
# With BRAGI_E2E_CERT_ONLY=1, node2 takes only a cert-only link and Bragi sends one (no seed).
LINK_ARGS=(); [[ ${BRAGI_E2E_CERT_ONLY:-} == 1 ]] && LINK_ARGS=(--cert-only)
( sleep 8; echo y ) | YMIRCTL "$N2" pool link "${LINK_ARGS[@]}" > "$ROOT/link2.out" 2>&1 &
LPID=$!; PIDS+=($LPID)
wait_for 60 "node2's link code" grep -q "weirdnet-ymir://link/" "$ROOT/link2.out"
sed -n 's/^ *\(weirdnet-ymir:\/\/link\/[^ ]*\)$/\1/p' "$ROOT/link2.out" | head -1 > "$WORK/ymir2.link"
wait_for 60 "Bragi's check code for node2" test -s "$WORK/bragi.accept.sas"
wait_for 60 "node2's check prompt" grep -q "Does the other device show" "$ROOT/link2.out"
YSAS2=$(sed -n 's/.*Does the other device show \([0-9]*\)?.*/\1/p' "$ROOT/link2.out" | head -1)
[[ "$YSAS2" == "$(cat "$WORK/bragi.accept.sas")" ]] || fail "check codes differ for node2: ymirctl $YSAS2, Bragi $(cat "$WORK/bragi.accept.sas")"
ok "Bragi opened a weirdnet-ymir:// code; both showed the same check code $YSAS2"
echo go > "$WORK/confirm2"
wait_for 60 "node2 to be linked" grep -q "^Linked\( cert-only\)\? as" "$ROOT/link2.out" || fail "pool link did not finish: $(cat "$ROOT/link2.out")"
wait_for 60 "Bragi to finish" test -f "$WORK/bragi.done2"
USER2=$(YMIRCTL "$N2" pool devices | sed -n 's/^Identity \([^ ]*\).*/\1/p')
[[ -n $USER1 && "$USER1" == "$USER2" ]] || fail "node2 joined a different identity ($USER1 vs $USER2)"
ok "node2 was linked by Bragi (direction 2) into the same identity"
if [[ ${BRAGI_E2E_CERT_ONLY:-} == 1 ]]; then
  [[ ! -e "$N2/state/pool/seed" && -f "$N2/state/pool/credential.json" ]] || fail "node2 holds a seed after a cert-only link"
  grep -q "^Linked cert-only as" "$ROOT/link2.out" || fail "node2 was not linked cert-only"
  ok "node2 holds no seed (cert-only)"
fi

# 4. node2 joins Weirdnet; the roster change reaches node1 and node1 sees node2.
start_ymird "$N2"
wait_for 60 "node2 on Weirdnet" node_addr "$N2"
wait_for 90 "node1's roster to list node2" sh -c "'$YMIR_BIN/ymirctl' --home '$N1' pool devices | grep -q 'Ymir on node2'"
ok "node1's roster lists node2, linked through Bragi"
echo "E2E PASSED (state: $ROOT)"
rm -rf "$ROOT"
