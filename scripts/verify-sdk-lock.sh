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

# The existing CI and release checks call this script. Declarative capabilities
# can require a newer Core schema without changing the backend's SDK transport.
# Keep Python tooling isolated from the machine's packages and pin its versions.
CONTRACT_VALIDATOR_DIR="$ROOT_DIR/.artifacts/contract-validator"
if [[ ! -f "$CONTRACT_VALIDATOR_DIR/pyvenv.cfg" ]]; then
    python3 -m venv "$CONTRACT_VALIDATOR_DIR"
fi
CONTRACT_PYTHON="$CONTRACT_VALIDATOR_DIR/bin/python"
if [[ ! -x "$CONTRACT_PYTHON" ]]; then
    CONTRACT_PYTHON="$CONTRACT_VALIDATOR_DIR/Scripts/python.exe"
fi
"$CONTRACT_PYTHON" -m pip install --quiet --disable-pip-version-check 'jsonschema==4.26.0' 'referencing==0.37.0'
"$CONTRACT_PYTHON" "$ROOT_DIR/scripts/verify-contract.py" "$@"
