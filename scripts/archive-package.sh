#!/bin/zsh

set -euo pipefail

# Archives a package directory built by scripts/build-package.sh into the
# unsigned plugin archive that Lithe Store lists. The package carries no
# signature; plugin.json declares vendor.signatureRequirement "none".
#
# The archive is dist/Lithe-Java-Support-<version>-darwin-<arm64|x64>.zip,
# created with `ditto --keepParent` so it holds the package directory itself.
# Development and release builds use the same archive. The archive is
# extracted again and re-verified, and dist/<asset>.sha256 records its
# SHA-256. Prints "<sha256>  dist/<asset>".

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="io.github.lithe-idea.java-support"
PACKAGE=""
ARCH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PACKAGE="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        *) print -u2 -- "Usage: $0 --package directory --arch arm64|x86_64"; exit 2 ;;
    esac
done
[[ -d "$PACKAGE" && "${PACKAGE:A:t}" == "$PLUGIN_ID" ]] || {
    print -u2 -- "--package must name a $PLUGIN_ID package directory"
    exit 2
}
case "$ARCH" in
    arm64) platform="darwin-arm64" ;;
    x86_64) platform="darwin-x64" ;;
    *) print -u2 -- "Unsupported architecture: $ARCH"; exit 2 ;;
esac

version=$(/usr/bin/plutil -extract version raw -o - "$PACKAGE/plugin.json")
asset="Lithe-Java-Support-$version-$platform.zip"

dist="$ROOT_DIR/dist"
mkdir -p "$dist"
rm -f "$dist/$asset" "$dist/$asset.sha256"
/usr/bin/ditto -c -k --keepParent "$PACKAGE" "$dist/$asset"

# Verify what users download, not the directory it was made from.
# The check directory stays under .artifacts with the other build outputs
# (.gitignore excludes it) instead of $TMPDIR.
extracted="$ROOT_DIR/.artifacts/archive-check/$platform"
rm -rf "$extracted"
mkdir -p "$extracted"
trap 'rm -rf "$extracted"' EXIT
/usr/bin/ditto -x -k "$dist/$asset" "$extracted"
entries=("$extracted"/*(DN:t))
[[ "${entries[*]}" == "$PLUGIN_ID" ]] || {
    print -u2 -- "$asset must contain only $PLUGIN_ID/, found: ${entries[*]}"
    exit 1
}
"$ROOT_DIR/scripts/verify-package.sh" --package "$extracted/$PLUGIN_ID" --arch "$ARCH"

checksum=$(/usr/bin/shasum -a 256 "$dist/$asset" | cut -d' ' -f1)
print -r -- "$checksum" > "$dist/$asset.sha256"
print -r -- "$checksum  dist/$asset"
