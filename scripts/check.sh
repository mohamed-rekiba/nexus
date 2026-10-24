#!/usr/bin/env bash
# The check for nexus/: format, lint and test everything.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
