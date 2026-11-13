#!/usr/bin/env bash
# Cargo verifies the exact SDK git revision; no matching release archive exists yet.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ $# -gt 1 ]]; then
    echo "Usage: $0 [--download]" >&2
    exit 2
fi
case "${1:-}" in
    "") ;;
    --download) cargo fetch --locked --manifest-path "$ROOT_DIR/Cargo.toml" ;;
    *) echo "Usage: $0 [--download]" >&2; exit 2 ;;
esac
cargo test --locked --manifest-path "$ROOT_DIR/Cargo.toml" -p lithe-java-backend \
    --test repository_pins cargo_resolves_the_sdk_commit_recorded_in_the_lock
