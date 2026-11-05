#!/bin/zsh

set -euo pipefail

# Builds the Java Support package directory for one architecture: copies
# plugin.json, assembles the pinned third-party resources and the plugin
# backend (the Rust binary lithe-java-backend, compiled with cargo for the
# matching Apple target, so the Rust toolchain and that target must be
# installed) next to it (scripts/assemble-package.sh) and verifies the result
# (scripts/verify-package.sh). Prints the package directory,
# .artifacts/package/<arch>/io.github.lithe-idea.java-support.
#
# This only builds the directory. Archiving it into the release asset is the
# separate step scripts/archive-package.sh.

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="io.github.lithe-idea.java-support"
ARCH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        *) print -u2 -- "Usage: $0 --arch arm64|x86_64"; exit 2 ;;
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

"$ROOT_DIR/scripts/verify-package.sh" --package "$package_dir" --arch "$ARCH"
print -r -- "$package_dir"
