#!/usr/bin/env bash
set +x
set -euo pipefail
umask 077
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
mkdir -p -- "$ROOT/target/raiv-validation"
TEST=$(mktemp -d "$ROOT/target/raiv-validation/run-tests.XXXXXX")
NATIVE_BUILD=${CARGO_TARGET_DIR:-$ROOT/target}
[[ $NATIVE_BUILD == /* ]] || NATIVE_BUILD="$PWD/$NATIVE_BUILD"
mkdir -p "$TEST/bin" "$TEST/build/debug" "$TEST/build/wasm32v1-none/release"
export LAUNCHER_TEST_TRACE="$TEST/trace"
export CARGO_TARGET_DIR="$TEST/build"
export PATH="$TEST/bin:$PATH"
unset OPENCODE_API_KEY
: > "$LAUNCHER_TEST_TRACE"
cat > "$TEST/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -eu
[[ ! ${OPENCODE_API_KEY+x} ]] || exit 91
[[ ! ${EXAMPLE_PROVIDER_KEY+x} ]] || exit 93
printf 'cargo:%s\n' "$1" >> "$LAUNCHER_TEST_TRACE"
EOF
cat > "$TEST/build/debug/raiv-agent" <<'EOF'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$1" >> "$LAUNCHER_TEST_TRACE"
config=${@: -1}
session=${config%/*}
case "$1" in
    init)
        printf 'main' > "$session/blocks/raiv-manifest.cbor"
        ;;&
    init|status)
        printf 'root=bafyreifg4eef2j33kx4epxkh72suj2lyamprn67hgksobxzvpxrqma5mtm\nrevision=0\nphase=AwaitActor\nadmissions=0\nunresolved=%s\npending_action_id=none\n' "${LAUNCHER_TEST_UNRESOLVED:-0}"
        ;;
    init-world)
        printf 'world' > "$session/world/raiv-world.cbor"
        ;;&
    init-world|world-status)
        printf 'world_digest=%064d\nvirtual_units_initial=10\nvirtual_units_available=10\nledger_rows=0\n' 0
        ;;
    interactive)
        [[ $# == 9 && $2 == --grants && $4 == --pin && $6 == --world-pin && $8 == --config ]]
        printf 'mock-interactive=started\n'
        ;;
    *) exit 92 ;;
esac
EOF
printf 'mock-guest' > "$TEST/build/wasm32v1-none/release/raiv_agent.wasm"
chmod +x "$TEST/bin/cargo" "$TEST/build/debug/raiv-agent"
expect_fail() {
    if "$@" > "$TEST/refusal.log" 2>&1; then printf 'Expected refusal: %s\n' "$1" >&2; exit 1; fi
}
pty() {
    local answer=$1 session=$2 log=$3 command
    command=$(printf '%q ' bash "$ROOT/run.sh" "$session")
    printf '%s\n' "$answer" | TERM=xterm timeout 15s script -q -e -c "$command" /dev/null > "$log" 2>&1
}
bash "$ROOT/run.sh" --help > "$TEST/help.log"
expect_fail bash "$ROOT/run.sh" --unknown
expect_fail bash "$ROOT/run.sh" --build unexpected-directory
bash "$ROOT/run.sh" --build > "$TEST/build.log" 2>&1
expect_fail bash "$ROOT/run.sh" "$TEST/non-tty"
[[ ! -e $TEST/non-tty ]]
# A terminal alone does not authorize startup without an environment credential.
if pty run "$TEST/no-key" "$TEST/no-key.log"; then exit 1; fi
[[ ! -e $TEST/no-key ]]
export OPENCODE_API_KEY=launcher-private-test-token
SESSION="$TEST/session with spaces"
pty run "$SESSION" "$TEST/first.log"
grep -q 'mock-interactive=started' "$TEST/first.log"
grep -Fxq 'max_tokens = 4096' "$SESSION/config.toml"
grep -Fxq 'timeout_ms = 60000' "$SESSION/config.toml"
grep -q 'New-session defaults: var/raiv/chat; 20 prompts/launch' "$TEST/help.log"
grep -q 'Plan: 20 prompts, <=3 calls/prompt, 4096 tokens/60s per call, actions=0.' "$TEST/first.log"
grep -Fxq 'remaining_model_calls = "3"' "$SESSION/grants.toml"
grep -Fxq 'remaining_actions = "0"' "$SESSION/grants.toml"
grep -Fxq 'max_events = "20"' "$SESSION/grants.toml"
BEFORE=$(sha256sum < "$SESSION/blocks/raiv-manifest.cbor")
pty run "$SESSION" "$TEST/reopen.log"
[[ $(grep -c '^init$' "$LAUNCHER_TEST_TRACE") == 1 && $(grep -c '^init-world$' "$LAUNCHER_TEST_TRACE") == 1 ]]
[[ $(sha256sum < "$SESSION/blocks/raiv-manifest.cbor") == "$BEFORE" ]]
export LAUNCHER_TEST_UNRESOLVED=1
if pty run "$SESSION" "$TEST/unresolved.log"; then exit 1; fi
unset LAUNCHER_TEST_UNRESOLVED
[[ $(grep -c '^interactive$' "$LAUNCHER_TEST_TRACE") == 2 ]]
if pty no "$SESSION" "$TEST/declined.log"; then exit 1; fi
[[ $(grep -c '^interactive$' "$LAUNCHER_TEST_TRACE") == 2 ]]
# A newer build never rebinds or orphans a session: it keeps running its own pinned guest.
printf 'changed-guest' > "$TEST/build/wasm32v1-none/release/raiv_agent.wasm"
pty run "$SESSION" "$TEST/changed-guest.log"
grep -q 'keeps its own pinned guest' "$TEST/changed-guest.log"
grep -q 'mock-interactive=started' "$TEST/changed-guest.log"
cmp -s <(printf 'mock-guest') "$SESSION/guest.wasm"
printf 'mock-guest' > "$TEST/build/wasm32v1-none/release/raiv_agent.wasm"
# The approval prompt shows the session's own plan, never the launcher's newer defaults.
printf '%s\n' 'activation_units = "100"' 'remaining_episode_units = "100"' \
    'remaining_model_calls = "3"' 'remaining_actions = "0"' 'max_events = "2"' > "$SESSION/grants.toml"
if pty no "$SESSION" "$TEST/older-plan.log"; then exit 1; fi
grep -q 'Plan: 2 prompts, <=3 calls/prompt' "$TEST/older-plan.log"
[[ $(grep -c '^interactive$' "$LAUNCHER_TEST_TRACE") == 3 ]]
FRESH_DECLINED="$TEST/fresh-declined"
if pty no "$FRESH_DECLINED" "$TEST/fresh-declined.log"; then exit 1; fi
[[ -f $FRESH_DECLINED/blocks/raiv-manifest.cbor && ! -e $FRESH_DECLINED/world/raiv-world.cbor ]]
if pty run "$FRESH_DECLINED" "$TEST/incomplete.log"; then exit 1; fi
[[ $(grep -c '^init$' "$LAUNCHER_TEST_TRACE") == 2 && $(grep -c '^init-world$' "$LAUNCHER_TEST_TRACE") == 1 ]]
printf '# changed\n' >> "$SESSION/config.toml"
if pty run "$SESSION" "$TEST/changed-config.log"; then exit 1; fi
# DATA inspection is independent of the launcher guard, credential, approval and TTY.
unset OPENCODE_API_KEY
bash "$ROOT/run.sh" --status "$SESSION" > "$TEST/status.log"
[[ $(grep -c '^interactive$' "$LAUNCHER_TEST_TRACE") == 3 ]]
# A NEW session may select any OpenAI-compatible endpoint with its own credential variable.
export OPENCODE_API_KEY=launcher-private-test-token RAIV_PROVIDER=openai-compatible \
    RAIV_BASE_URL=https://api.example.com/v1 RAIV_MODEL=qwen3.8-flash RAIV_API_KEY_ENV=EXAMPLE_PROVIDER_KEY
GENERIC="$TEST/generic"
if pty run "$GENERIC" "$TEST/generic-no-key.log"; then exit 1; fi
[[ ! -e $GENERIC ]]
grep -q 'Export EXAMPLE_PROVIDER_KEY first' "$TEST/generic-no-key.log"
export EXAMPLE_PROVIDER_KEY=generic-private-test-token
RAIV_BASE_URL='https://api.example.com/v1?key=x' pty run "$TEST/generic-bad-url" "$TEST/generic-bad-url.log" && exit 1
pty run "$GENERIC" "$TEST/generic.log"
grep -q 'mock-interactive=started' "$TEST/generic.log"
grep -q 'Raiv — openai-compatible / qwen3.8-flash (LOCAL TEST)' "$TEST/generic.log"
grep -Fxq 'provider = "openai-compatible"' "$GENERIC/config.toml"
grep -Fxq 'api_key_env = "EXAMPLE_PROVIDER_KEY"' "$GENERIC/config.toml"
grep -Fxq 'model = "qwen3.8-flash"' "$GENERIC/config.toml"
if grep -q 'reasoning_effort' "$GENERIC/config.toml"; then exit 1; fi
# Reopening uses the session's own key variable even when the new-session selection changes.
unset RAIV_PROVIDER RAIV_BASE_URL RAIV_MODEL RAIV_API_KEY_ENV OPENCODE_API_KEY
pty run "$GENERIC" "$TEST/generic-reopen.log"
grep -q 'mock-interactive=started' "$TEST/generic-reopen.log"
unset EXAMPLE_PROVIDER_KEY
# Selecting only the provider uses the OpenCode endpoint, qwen3.8-flash and OPENCODE_API_KEY.
export RAIV_PROVIDER=openai-compatible OPENCODE_API_KEY=launcher-private-test-token
DEFAULTS="$TEST/generic-defaults"
pty run "$DEFAULTS" "$TEST/generic-defaults.log"
grep -q 'mock-interactive=started' "$TEST/generic-defaults.log"
grep -Fxq 'base_url = "https://opencode.ai/zen/go/v1"' "$DEFAULTS/config.toml"
grep -Fxq 'model = "qwen3.8-flash"' "$DEFAULTS/config.toml"
grep -Fxq 'api_key_env = "OPENCODE_API_KEY"' "$DEFAULTS/config.toml"
unset RAIV_PROVIDER OPENCODE_API_KEY
if grep -R -F -q -e 'launcher-private-test-token' -e 'generic-private-test-token' "$TEST" --exclude='test-run.sh'; then
    printf 'Credential leaked into test output/files.\n' >&2; exit 1
fi
printf 'Launcher shell regressions passed; evidence: %s\n' "$TEST"

# Optional actual-Wasmi startup/exit with no Enter in the UI and no credential in the host.
if [[ ${1:-} == --native ]]; then
    [[ -x $NATIVE_BUILD/debug/raiv-agent && -f $NATIVE_BUILD/wasm32v1-none/release/raiv_agent.wasm ]]
    export LAUNCHER_NATIVE_HOST="$NATIVE_BUILD/debug/raiv-agent"
    cp -- "$NATIVE_BUILD/wasm32v1-none/release/raiv_agent.wasm" "$TEST/build/wasm32v1-none/release/raiv_agent.wasm"
    cat > "$TEST/build/debug/raiv-agent" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
unset OPENCODE_API_KEY
exec "$LAUNCHER_NATIVE_HOST" "$@"
EOF
    chmod +x "$TEST/build/debug/raiv-agent"
    export OPENCODE_API_KEY=launcher-private-test-token
    SESSION="$TEST/native session"
    COMMAND=$(printf '%q ' bash "$ROOT/run.sh" "$SESSION")
    (printf 'run\n'; sleep 2; printf '\033'; sleep 1) | \
        TERM=xterm timeout 20s script -q -e -c "$COMMAND" /dev/null > "$TEST/native-pty.log" 2>&1
    unset OPENCODE_API_KEY
    bash "$ROOT/run.sh" --status "$SESSION" > "$TEST/native-status.log"
    grep -Fxq 'revision=0' "$TEST/native-status.log"
    grep -Fxq 'admissions=0' "$TEST/native-status.log"
    grep -Fxq 'unresolved=0' "$TEST/native-status.log"
    grep -Fxq 'virtual_units_available=10' "$TEST/native-status.log"
    grep -Fxq 'ledger_rows=0' "$TEST/native-status.log"
    if grep -R -F -q 'launcher-private-test-token' "$SESSION" "$TEST/native-pty.log"; then exit 1; fi
    printf 'Actual-Wasmi launcher startup/ESC exit passed; zero admissions, no host credential/network.\n'
fi
