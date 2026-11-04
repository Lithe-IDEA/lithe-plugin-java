#!/bin/zsh

set -euo pipefail

# Builds the unsigned Java Support package directory for one architecture:
# copies plugin.json, assembles the pinned third-party resources next to it
# (scripts/assemble-package.sh) and verifies the result
# (scripts/verify-package.sh). Prints the package directory,
# .artifacts/package/<arch>/dev.lithe.plugin.java-support.
#
# Signing and archiving are a separate step (scripts/archive-package.sh) so
# that the release workflow only exposes the signing key to that step.

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="dev.lithe.plugin.java-support"
ARCH=""
HOST_TOOLS=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        --host-tools) HOST_TOOLS="$2"; shift 2 ;;
        *) print -u2 -- "Usage: $0 --arch arm64|x86_64 [--host-tools bin-directory]"; exit 2 ;;
    esac
done
# Each release asset targets one macOS architecture; the store lists them as
# darwin-arm64 and darwin-x64.
case "$ARCH" in
    arm64|x86_64) ;;
    *) print -u2 -- "Unsupported architecture: $ARCH"; exit 2 ;;
esac

package_dir="$ROOT_DIR/.artifacts/package/$ARCH/$PLUGIN_ID"
rm -rf "$package_dir"
mkdir -p "$package_dir"
cp "$ROOT_DIR/$PLUGIN_ID/plugin.json" "$package_dir/plugin.json"
"$ROOT_DIR/scripts/assemble-package.sh" --output "$package_dir" --arch "$ARCH" >&2

verify_args=(--package "$package_dir" --arch "$ARCH")
[[ -z "$HOST_TOOLS" ]] || verify_args+=(--host-tools "$HOST_TOOLS")
"$ROOT_DIR/scripts/verify-package.sh" "${verify_args[@]}"
print -r -- "$package_dir"
