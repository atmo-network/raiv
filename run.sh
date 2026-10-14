#!/usr/bin/env bash
set +x
set -euo pipefail
umask 077

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
MODE=run
case "${1:-}" in
    -h|--help)
        printf '%s\n' 'Usage: ./run.sh [session-directory]' \
            '       ./run.sh --status [session-directory]' \
            '       ./run.sh --build' \
            'Linux interactive Go/high demo. Defaults: var/raiv/chat; 1024 tokens, 2 prompts, <=6 calls, no actions.' \
            'Requires OPENCODE_API_KEY in the environment. No inference until Enter in the UI.'
        exit 0 ;;
    --build|--status) MODE=${1#--}; shift ;;
    -*) printf 'Unknown option. Use --help.\n' >&2; exit 2 ;;
esac
(( $# <= 1 )) || { printf 'Too many arguments. Use --help.\n' >&2; exit 2; }
[[ $MODE != build || $# == 0 ]] || { printf '--build takes no directory.\n' >&2; exit 2; }
SESSION=${1:-$ROOT/var/raiv/chat}
[[ $SESSION == /* ]] || SESSION="$PWD/$SESSION"
BUILD=${CARGO_TARGET_DIR:-$ROOT/target}
[[ $BUILD == /* ]] || BUILD="$PWD/$BUILD"
HOST="$BUILD/debug/raiv-agent"
MODULE="$BUILD/wasm32v1-none/release/raiv_agent.wasm"
CONFIG="$SESSION/config.toml"
PLAN='activation_units = "100"
remaining_episode_units = "100"
remaining_model_calls = "3"
remaining_actions = "0"
max_events = "2"'
fail() { printf '%s\n' "$*" >&2; exit 1; }
digest() { sha256sum < "$1" | cut -d ' ' -f 1; }
field() {
    local line value='' count=0
    while IFS= read -r line; do
        if [[ $line == "$1="* ]]; then value=${line#*=}; count=$((count + 1)); fi
    done <<< "$2"
    [[ $count == 1 ]] || fail "Missing/duplicate native status field: $1"
    printf '%s\n' "$value"
}
cd -- "$ROOT"
trap 'printf "Stopped. No automatic retry or repair. Session: %s\n" "$SESSION" >&2' ERR
if [[ $MODE == status ]]; then
    [[ -x $HOST && -f $CONFIG ]] || fail 'Session or host missing; use --build for the host. No initialization performed.'
    unset OPENCODE_API_KEY
    "$HOST" status --config "$CONFIG"
    "$HOST" world-status --config "$CONFIG"
    exit 0
fi
if [[ $MODE == run ]]; then
    [[ -t 0 && -t 1 ]] || fail 'Interactive launch requires a terminal (stdin and stdout).'
    [[ -n ${OPENCODE_API_KEY:-} ]] || fail 'Export OPENCODE_API_KEY first; never put the value in config or arguments.'
fi
# Build offline, without exposing the credential to Cargo/build scripts.
(
    unset OPENCODE_API_KEY
    export CARGO_TARGET_DIR="$BUILD"
    cargo rustc --offline --locked -p raiv-agent-guest --lib --crate-type cdylib \
        --target wasm32v1-none --release -- -C panic=abort
    cargo build --offline --locked -p raiv-agent-host --bin raiv-agent
)
[[ $MODE != build ]] || exit 0
[[ ! -L $SESSION ]] || fail 'Session directory must not be a symlink.'
FRESH=false
if [[ ! -e $SESSION ]]; then
    mkdir -p -- "$(dirname -- "$SESSION")"
    mkdir -- "$SESSION"
    FRESH=true
    mkdir -- "$SESSION/blocks" "$SESSION/world"
    cp -- "$MODULE" "$SESSION/guest.wasm"
    AGENT=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
    WORLD=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
    cat > "$CONFIG" <<EOF
schema_version = 1
state_format = "raiv-state/1"
[agent]
profile = "survivor"
id = "$AGENT"
mandate = "Assist the local operator. Preserve useful behavioral continuity with patch_memory when needed, then finish with a complete normal response without tool calls. External actions are not approved."
[execution]
adapter = "local-trusted"
[runtime]
guest_module = "guest.wasm"
module_digest = "$(digest "$SESSION/guest.wasm")"
[inference]
protocol = "openai-chat-completions"
base_url = "https://opencode.ai/zen/go/v1"
model = "deepseek-v4.1-flash"
reasoning_effort = "high"
api_key_env = "OPENCODE_API_KEY"
stream = true
max_tokens = 1024
max_request_bytes = 262144
max_response_bytes = 1048576
max_header_bytes = 16384
timeout_ms = 20000
max_stream_event_bytes = 65536
max_stream_events = 4096
[storage]
adapter = "disk"
directory = "blocks"
[acceptance]
adapter = "local"
[executor]
adapter = "scripted-unit-world"
directory = "world"
world_id = "$WORLD"
virtual_units = "10"
capacity = 16
EOF
    printf '%s\n' "$PLAN" > "$SESSION/grants.toml"
    digest "$CONFIG" > "$SESSION/config.sha256"
    # Explicit fresh revision-zero administration only; never initialize an existing namespace.
    STATUS=$("$HOST" init --config "$CONFIG")
else
    [[ -f $CONFIG && -f $SESSION/config.sha256 && -f $SESSION/grants.toml && \
        -f $SESSION/guest.wasm && -f $SESSION/blocks/raiv-manifest.cbor && \
        -f $SESSION/world/raiv-world.cbor ]] || fail "Incomplete session: $SESSION. Inspect it; no automatic bootstrap/repair."
    [[ $(digest "$CONFIG") == "$(< "$SESSION/config.sha256")" ]] || fail 'Configuration changed; use a separately approved NEW directory, never rebind this session.'
    cmp -s -- "$MODULE" "$SESSION/guest.wasm" || fail 'Guest changed; keep the old session intact and choose a separately approved NEW directory.'
    cmp -s -- <(printf '%s\n' "$PLAN") "$SESSION/grants.toml" || fail 'Grant plan changed; launch refused rather than widening limits.'
    STATUS=$("$HOST" status --config "$CONFIG")
fi
[[ $(field unresolved "$STATUS") == 0 && $(field phase "$STATUS") == AwaitActor && \
    $(field pending_action_id "$STATUS") == none ]] || fail 'Unresolved admission or pending action: inspect independently; never relaunch or dispatch here.'
MAIN=$(digest "$SESSION/blocks/raiv-manifest.cbor")
REVISION=$(field revision "$STATUS")
ROOT_CID=$(field root "$STATUS")
printf '\nRaiv — Go / deepseek-v4.1-flash / high (LOCAL TEST)\nSession: %s\nConfig SHA-256: %s\nGuest SHA-256: %s\nRoot: %s\nRevision: %s\nMain manifest SHA-256: %s\n' \
    "$SESSION" "$(digest "$CONFIG")" "$(digest "$SESSION/guest.wasm")" "$ROOT_CID" "$REVISION" "$MAIN"
if $FRESH; then
    [[ $REVISION == 0 && $(field admissions "$STATUS") == 0 ]] || fail 'Fresh source is not an empty genesis.'
    printf 'World: NEW empty synthetic namespace, id=%s, 10 units, no effects.\n' "$WORLD"
else
    WORLD_STATUS=$("$HOST" world-status --config "$CONFIG")
    WORLD_DIGEST=$(field world_digest "$WORLD_STATUS")
    printf '%s\n' "$WORLD_STATUS"
fi
printf 'Plan: 2 prompts, <=3 calls/prompt (<=6 total), 1024 tokens/20s per call, actions=0.\nSynthetic grants: 100 activation/episode units per event; NOT real billing or lifetime funding.\n'
printf 'Approve this exact source/config/plan%s? Type run (anything else stops): ' "$($FRESH && printf ' and its NEW empty world bootstrap' || true)"
IFS= read -r ANSWER || exit 1
[[ $ANSWER == run ]] || fail 'Not approved. Session retained; no inference or automatic retry.'
printf 'root = "%s"\nrevision = "%s"\nmanifest_digest = "%s"\n' \
    "$ROOT_CID" "$REVISION" "$MAIN" > "$SESSION/approved-main.toml"
if $FRESH; then
    # Carry only the result of this approved genesis transaction, never discover an old head.
    WORLD_STATUS=$("$HOST" init-world --pin "$SESSION/approved-main.toml" --config "$CONFIG")
    WORLD_DIGEST=$(field world_digest "$WORLD_STATUS")
fi
printf 'Enter submits one approved prompt; Esc exits after bounded inference. Memory stays in this directory.\n'
exec "$HOST" interactive --grants "$SESSION/grants.toml" --pin "$SESSION/approved-main.toml" \
    --world-pin "$WORLD_DIGEST" --config "$CONFIG"
