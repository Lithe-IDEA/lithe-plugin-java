#!/bin/zsh

set -euo pipefail

# Builds the Lithe package tools this plugin is signed and verified with:
# LithePluginPackageSigner (it holds the trusted publisher keys) and
# LitheOfficialPluginVerifier (it applies the host's plugin contract). Both
# come from the Lithe revision pinned in host-tools.lock.json, fetched into
# .artifacts/host-tools/<revision>/source; nothing outside this repository is
# written. Prints the directory that holds the two executables.
#
# LITHE_HOST_SOURCE may name a local Lithe clone to fetch the revision from
# instead of the network; the clone is only read.
#
# The pinned revision names its Swift compiler in .swift-version. A different
# compiler only prints a warning, unless --require-pinned-swift is given (CI
# and releases use it).

ROOT_DIR="${0:A:h:h}"
LOCK="$ROOT_DIR/host-tools.lock.json"
REQUIRE_PINNED_SWIFT=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --require-pinned-swift) REQUIRE_PINNED_SWIFT=true; shift ;;
        *) print -u2 -- "Usage: $0 [--require-pinned-swift]"; exit 2 ;;
    esac
done

repository=$(/usr/bin/plutil -extract repository raw -o - "$LOCK")
revision=$(/usr/bin/plutil -extract revision raw -o - "$LOCK")
[[ "$revision" =~ '^[0-9a-f]{40}$' ]] || {
    print -u2 -- "host-tools.lock.json must pin a full commit: $revision"
    exit 1
}
work="$ROOT_DIR/.artifacts/host-tools/$revision"
source_dir="$work/source"

if [[ "$(git -C "$source_dir" rev-parse HEAD 2>/dev/null || true)" != "$revision" ]]; then
    rm -rf "$source_dir"
    mkdir -p "$source_dir"
    git -C "$source_dir" init --quiet
    git -C "$source_dir" fetch --quiet --depth 1 "${LITHE_HOST_SOURCE:-$repository}" "$revision"
    git -C "$source_dir" checkout --quiet --detach FETCH_HEAD
fi
[[ "$(git -C "$source_dir" rev-parse HEAD)" == "$revision" ]] || {
    print -u2 -- "Fetched Lithe source is not $revision"
    exit 1
}

expected_swift=$(<"$source_dir/.swift-version")
actual_swift=$(swift --version 2>/dev/null | sed -nE 's/.*Swift version ([0-9]+\.[0-9]+(\.[0-9]+)?).*/\1/p' | head -n 1)
if [[ "$actual_swift" != "$expected_swift" ]]; then
    message="Lithe $revision builds with Swift $expected_swift, found ${actual_swift:-no Swift}"
    [[ "$REQUIRE_PINNED_SWIFT" == false ]] || { print -u2 -- "$message"; exit 1; }
    print -u2 -- "warning: $message"
fi

case "$(uname -m)" in
    arm64) triple="arm64-apple-macosx" ;;
    x86_64) triple="x86_64-apple-macosx" ;;
    *) print -u2 -- "Unsupported host architecture: $(uname -m)"; exit 1 ;;
esac
# Same flags Lithe uses for these products; building them alone keeps the
# application and its Rust Core out of the build.
swift_args=(
    --package-path "$source_dir"
    --scratch-path "$work/build"
    --triple "$triple"
    -Xswiftc -Xfrontend
    -Xswiftc -disable-round-trip-debug-types
)
for product in LithePluginPackageSigner LitheOfficialPluginVerifier; do
    swift build "${swift_args[@]}" --product "$product" >&2
done
bin_dir=$(swift build "${swift_args[@]}" --show-bin-path)
for product in LithePluginPackageSigner LitheOfficialPluginVerifier; do
    [[ -x "$bin_dir/$product" ]] || {
        print -u2 -- "Lithe host tool was not built: $bin_dir/$product"
        exit 1
    }
done
print -r -- "$bin_dir"
