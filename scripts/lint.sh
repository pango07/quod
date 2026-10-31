#!/usr/bin/env bash
# fmt, clippy -D warnings, unit tests. Fails loudly; run before every commit.
set -euo pipefail
cargo fmt --all --check
cargo clippy -q --workspace --all-targets -- -D warnings
cargo test -q --workspace >/dev/null
echo lint ok
