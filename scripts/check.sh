#!/usr/bin/env bash
# The check for nexus/: format, lint and test everything.
#   scripts/check.sh        all of it
#   scripts/check.sh lint   the format check and clippy
#   scripts/check.sh test   the tests
# CI runs the same commands as separate jobs (.github/workflows/ci.yml).
set -euo pipefail
cd "$(dirname "$0")/.."

part=${1:-all}
case "$part" in all | lint | test) ;; *) echo "usage: $0 [lint|test]" >&2; exit 2 ;; esac

if [ "$part" != test ]; then
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
fi

if [ "$part" != lint ]; then
    # A failing test binary doesn't hide the failures of the ones after it.
    if cargo nextest --version >/dev/null 2>&1; then
        # Every test in its own process, all of them at once. nextest leaves out doc tests;
        # the code has none that run.
        cargo nextest run --workspace --no-fail-fast
    else
        cargo test --workspace --no-fail-fast
    fi
fi
