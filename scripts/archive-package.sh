#!/bin/zsh

set -euo pipefail

# Signs and archives a package directory built by scripts/build-package.sh.
#
# --sign reads the publisher key from LITHE_PLUGIN_PACKAGE_PRIVATE_KEY and
# signs with the Lithe signer from scripts/host-tools.sh; only the release
# workflow's publishing step has that key. --development-unsigned archives
# the package without a signature; only development builds of Lithe install
# such a package, so its file name says so.
#
# The archive is dist/Lithe-Java-Support-<version>-darwin-<arm64|x64>.zip
# (with -unsigned before .zip for development archives), created with
# `ditto --keepParent` so it holds the package directory itself. The archive
# is extracted again and re-verified, and dist/<asset>.sha256 records its
# SHA-256. Prints "<sha256>  dist/<asset>".

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="dev.lithe.plugin.java-support"
PACKAGE=""
ARCH=""
HOST_TOOLS=""
MODE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PACKAGE="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        --host-tools) HOST_TOOLS="$2"; shift 2 ;;
        --sign) MODE="sign"; shift ;;
        --development-unsigned) MODE="unsigned"; shift ;;
        *) print -u2 -- "Usage: $0 --package directory --arch arm64|x86_64 (--sign --host-tools bin-directory | --development-unsigned)"; exit 2 ;;
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
case "$MODE" in
    sign)
        [[ -n "$HOST_TOOLS" ]] || { print -u2 -- "--sign needs --host-tools"; exit 2; }
        [[ -n "${LITHE_PLUGIN_PACKAGE_PRIVATE_KEY:-}" ]] || {
            print -u2 -- "Configure LITHE_PLUGIN_PACKAGE_PRIVATE_KEY to sign the Java Support package"
            exit 1
        }
        rm -f "$PACKAGE/lithe-plugin-signature.json"
        print -rn -- "$LITHE_PLUGIN_PACKAGE_PRIVATE_KEY" | "$HOST_TOOLS/LithePluginPackageSigner" "$PACKAGE"
        asset="Lithe-Java-Support-$version-$platform.zip"
        verify_args=(--require-signature)
        ;;
    unsigned)
        [[ ! -f "$PACKAGE/lithe-plugin-signature.json" ]] || {
            print -u2 -- "The package is already signed; rebuild it before a development archive"
            exit 1
        }
        asset="Lithe-Java-Support-$version-$platform-unsigned.zip"
        verify_args=()
        ;;
    *) print -u2 -- "Choose --sign or --development-unsigned"; exit 2 ;;
esac
[[ -z "$HOST_TOOLS" ]] || verify_args+=(--host-tools "$HOST_TOOLS")

dist="$ROOT_DIR/dist"
mkdir -p "$dist"
rm -f "$dist/$asset" "$dist/$asset.sha256"
/usr/bin/ditto -c -k --keepParent "$PACKAGE" "$dist/$asset"

# Verify what users download, not the directory it was made from.
# The check directory stays inside the repository: the official verifier
# rejects packages under a symbolically linked path such as $TMPDIR
# (/var is /private/var on macOS).
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
"$ROOT_DIR/scripts/verify-package.sh" --package "$extracted/$PLUGIN_ID" --arch "$ARCH" "${verify_args[@]}"

checksum=$(/usr/bin/shasum -a 256 "$dist/$asset" | cut -d' ' -f1)
print -r -- "$checksum" > "$dist/$asset.sha256"
print -r -- "$checksum  dist/$asset"
