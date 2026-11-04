#!/usr/bin/env bash

# Checks that the backend links exactly the plugin SDK recorded in
# sdk.lock.json.
#
# Without arguments the check is offline: Cargo.toml must name the locked tag
# and Cargo.lock must resolve the locked commit (backend/tests/repository_pins.rs).
# With --download it also fetches the published crate archive into
# .artifacts/sdk/ and refuses it unless its SHA-256 matches the lock, which
# proves the tag still points at the release the lock was written for.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT_DIR/sdk.lock.json"
DOWNLOAD=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --download) DOWNLOAD=true; shift ;;
        *) echo "Usage: $0 [--download]" >&2; exit 2 ;;
    esac
done

cargo test --locked --manifest-path "$ROOT_DIR/Cargo.toml" -p lithe-java-backend \
    --test repository_pins cargo_resolves_the_sdk_commit_recorded_in_the_lock

[[ "$DOWNLOAD" == true ]] || exit 0

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

url="$(jq -r .url "$LOCK")"
expected="$(jq -r .sha256 "$LOCK")"
version="$(jq -r .version "$LOCK")"
destination="$ROOT_DIR/.artifacts/sdk/lithe-plugin-sdk-rust-$version-$expected.crate"
mkdir -p "$(dirname "$destination")"
if [[ ! -f "$destination" || "$(sha256 "$destination")" != "$expected" ]]; then
    temporary="$destination.download.$$"
    trap 'rm -f "$temporary"' EXIT
    curl --fail --location --silent --show-error --retry 3 \
        --connect-timeout 20 --max-time 300 --output "$temporary" "$url"
    actual="$(sha256 "$temporary")"
    if [[ "$actual" != "$expected" ]]; then
        echo "SDK archive checksum mismatch: expected $expected, got $actual ($url)" >&2
        exit 1
    fi
    mv "$temporary" "$destination"
fi
echo "Verified SDK archive $url"
