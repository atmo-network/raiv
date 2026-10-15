#!/usr/bin/env bash
# Opt-in live witness: the ordinary ./run.sh in an owned PTY keeps semantic Memory across
# two fresh UI/VM processes. Spends real inference; never part of CI or ordinary tests.
#   bash apps/raiv-agent/test-live-launcher.sh --check-offline
#   RAIV_LIVE_SMOKE=opencode-go/deepseek-v4.1-flash/launcher-memory-4096 bash apps/raiv-agent/test-live-launcher.sh
#   ... --continue-accepted var/raiv/launcher-witness-<id>   # only after a known accepted stage 1
set +x
set -euo pipefail
umask 077
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
APPROVAL=opencode-go/deepseek-v4.1-flash/launcher-memory-4096
NICK=ORION-RAIV-7
PROMPTS=(
    "Remember my nickname $NICK in intents.nickname using patch_memory, then answer briefly without further tool calls."
    "Read the saved intents.nickname from accepted Memory. Use patch_memory to copy exactly that existing value to tactics.recalled_nickname, preserving intents. Then answer briefly without further tool calls."
)
MEMORY=(
    "{\"intents\":{\"nickname\":\"$NICK\"}}"
    "{\"intents\":{\"nickname\":\"$NICK\"},\"tactics\":{\"recalled_nickname\":\"$NICK\"}}"
)
HOST="${CARGO_TARGET_DIR:-$ROOT/target}/debug/raiv-agent"
fail() { printf 'LAUNCHER_REFUSAL %s; raw output withheld; inspect the retained session, never retry automatically\n' "$*" >&2; exit 1; }
# Exactly one `key=value` line of native status output.
field() {
    local values
    values=$(grep -E "^$1=" <<< "$2") || fail "missing status field $1"
    [[ $(wc -l <<< "$values") == 1 ]] || fail "duplicate status field $1"
    printf '%s\n' "${values#*=}"
}
status() { env -u OPENCODE_API_KEY "$HOST" "$1" --config "$SESSION/config.toml"; }
# Raw terminal output stays in one private temporary directory, removed on any exit.
STAGE_DIR=
trap '[[ -z $STAGE_DIR ]] || rm -rf -- "$STAGE_DIR"' EXIT
wait_for() { timeout "$2" bash -c 'until grep -aqF -- "$1" "$2"; do sleep 0.2; done' _ "$1" "$3"; }

check_offline() {
    [[ ${PROMPTS[1]} != *"$NICK"* ]] || fail 'second prompt must not repeat the nickname'
    [[ ${PROMPTS[0]} == *patch_memory* && ${PROMPTS[1]} == *patch_memory* ]] || fail 'prompts must name patch_memory'
    grep -Fq "max_events = \"20\"" "$ROOT/run.sh" || fail 'launcher plan differs from this witness'
    grep -Fxq 'max_tokens = 4096' "$ROOT/run.sh" || fail 'launcher token cap differs from this witness'
    bash -n "$ROOT/run.sh"
    printf 'launcher witness offline checks passed; no credential/process/network\n'
}

# The session must be exactly what the ordinary launcher creates for the default provider.
configuration() {
    local config="$SESSION/config.toml" mandate
    mandate=$(grep -E '^mandate = ' "$ROOT/run.sh") || fail 'launcher mandate missing'
    grep -Fxq "$mandate" "$config" || fail 'session mandate differs from the launcher'
    for line in 'profile = "survivor"' 'model = "deepseek-v4.1-flash"' 'base_url = "https://opencode.ai/zen/go/v1"' \
        'reasoning_effort = "high"' 'max_tokens = 4096' 'timeout_ms = 60000' \
        'api_key_env = "OPENCODE_API_KEY"' 'stream = true' 'virtual_units = "10"'; do
        grep -Fxq "$line" "$config" || fail "session configuration lacks: $line"
    done
    [[ $(sha256sum < "$config" | cut -d ' ' -f 1) == "$(< "$SESSION/config.sha256")" ]] || fail 'configuration digest differs'
    grep -Fxq "module_digest = \"$(sha256sum < "$SESSION/guest.wasm" | cut -d ' ' -f 1)\"" "$config" || fail 'guest digest differs'
    cmp -s "$SESSION/grants.toml" <(printf '%s\n' 'activation_units = "100"' 'remaining_episode_units = "100"' \
        'remaining_model_calls = "3"' 'remaining_actions = "0"' 'max_events = "20"') || fail 'grant plan differs'
}

checked() {
    local n=$1 info world
    info=$(status status) || fail "status unavailable after stage $n"
    world=$(status world-status) || fail "world status unavailable after stage $n"
    [[ $(field unresolved "$info") == 0 ]] || fail "stage=$n unresolved admission"
    [[ $(field revision "$info") == "$n" && $(field phase "$info") == AwaitActor ]] || fail "stage=$n wrong revision/phase"
    [[ $(field admissions "$info") == "$n" && $(field consumed_admissions "$info") == "$n" ]] || fail "stage=$n admissions"
    [[ $(field completed_episodes "$info") == "$n" && $(field pending_action_id "$info") == none ]] || fail "stage=$n episodes"
    [[ $(field memory "$info") == "${MEMORY[n-1]}" ]] || fail "stage=$n semantic Memory differs"
    [[ $(field virtual_units_available "$world") == 10 && $(field ledger_rows "$world") == 0 ]] || fail "stage=$n world changed"
}

# One fresh launcher process: approve, submit one prompt, then Esc (honored after the boundary).
stage() {
    local n=$1 log fifo pid
    STAGE_DIR=$(mktemp -d)
    log="$STAGE_DIR/pty" fifo="$STAGE_DIR/input"
    mkfifo "$fifo"
    TERM=xterm timeout 240s script -q -e -f \
        -c "stty rows 30 cols 120; exec bash $(printf '%q %q' "$ROOT/run.sh" "$SESSION")" "$log" < "$fifo" > /dev/null &
    pid=$!
    exec 3> "$fifo"
    wait_for 'Type run (anything else stops):' 120 "$log" || fail "stage=$n no approval prompt"
    [[ -e $SESSION/config.toml ]] && configuration
    if (( n > 1 )); then
        local info
        info=$(status status) || fail 'status unavailable before approval'
        [[ $(field revision "$info") == $((n - 1)) && $(field unresolved "$info") == 0 ]] || fail "stage=$n source differs"
    fi
    printf 'run\n' >&3
    wait_for $'\e[?1049h' 30 "$log" && wait_for $'\e[?25h' 30 "$log" || fail "stage=$n no editable frame"
    sleep 1
    printf '\e[200~%s\e[201~\r' "${PROMPTS[n-1]}" >&3
    sleep 2
    printf '\e' >&3
    wait "$pid" || fail "stage=$n launcher exit status $?"
    exec 3>&-
    grep -aqF 'Agent' "$log" || fail "stage=$n no agent reply shown"
    rm -rf -- "$STAGE_DIR"
    STAGE_DIR=
    printf 'LAUNCHER_UI stage=%s approved=true submitted=1 fenced_exit=true process=0\n' "$n"
}

case "${1:-}" in
    --check-offline) check_offline; exit 0 ;;
    --continue-accepted) (( $# == 2 )) || fail 'usage'; RESUME=$2 ;;
    '') RESUME= ;;
    *) fail 'usage: test-live-launcher.sh [--check-offline|--continue-accepted <owned-session>]' ;;
esac
[[ ${RAIV_LIVE_SMOKE:-} == "$APPROVAL" ]] || fail 'explicit bounded launcher live selector required'
[[ -n ${OPENCODE_API_KEY:-} ]] || fail 'selected credential unavailable; value withheld'
check_offline > /dev/null
mkdir -p "$ROOT/var/raiv"
if [[ -n $RESUME ]]; then
    SESSION=$(cd -- "$RESUME" && pwd)
    [[ $SESSION == "$ROOT/var/raiv/launcher-witness-"* ]] || fail 'only an owned witness session can continue'
    configuration
    checked 1
else
    SESSION="$ROOT/var/raiv/launcher-witness-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
    [[ ! -e $SESSION ]] || fail 'witness namespace exists'
fi
printf 'LAUNCHER_HOME=%s\n' "$SESSION"
if [[ -z $RESUME ]]; then
    stage 1
    configuration
    checked 1
fi
printf 'LAUNCHER_PROOF stage=1 revision=1 semantic_memory=true\n'
WORLD=$(sha256sum < "$SESSION/world/raiv-world.cbor")
stage 2
checked 2
[[ $(sha256sum < "$SESSION/world/raiv-world.cbor") == "$WORLD" ]] || fail 'world bytes changed'
printf 'LAUNCHER_RESULT=passed stages=2 ordinary_bash=true fresh_ui_vm=true behavioral_recall=true no_effects=true\n'
