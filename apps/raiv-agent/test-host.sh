#!/bin/sh
# Bounded native/Wasmi/process test selections. No production code, publishing or effects.
set -eu
if [ "$#" -gt 1 ]; then
    echo "usage: sh apps/raiv-agent/test-host.sh [Rust-toolchain]" >&2
    exit 2
fi
: "${RAIV_GUEST_WASM:?build the guest first and select its exact module with RAIV_GUEST_WASM}"
# Resolve artifact before changing directory; never treat a missing module as passed.
case "$RAIV_GUEST_WASM" in /*) ;; *) RAIV_GUEST_WASM="$PWD/$RAIV_GUEST_WASM" ;; esac
export RAIV_GUEST_WASM
if [ ! -f "$RAIV_GUEST_WASM" ]; then
    echo "guest module missing: $RAIV_GUEST_WASM" >&2
    exit 2
fi
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/../.."
if [ "$#" -eq 1 ]; then set -- "+$1"; fi
cargo "$@" test --locked -p raiv-agent-host
cargo "$@" test --locked -p raiv-agent-host -- --ignored --skip disk::manifest::tests::process_ --skip owned_tty_ --skip live_provider_smoke
cargo "$@" test --locked -p raiv-agent-host --lib disk::manifest::tests::process_ -- --ignored --test-threads=1
