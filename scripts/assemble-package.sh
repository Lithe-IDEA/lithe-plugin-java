#!/bin/zsh

set -euo pipefail

# Assembles the resources of the Java Support plugin package: what the plugin
# adds to the user's own JDT LS (the Lombok agent and the Java Debug and Java
# Test extension bundles) and the plugin backend (the Rust binary
# lithe-java-backend, built here for the requested architecture and placed
# where plugin.json's backend.launch.path names it). JDT LS itself and the JDK
# it runs on are not packaged: the backend finds the user's installation
# (backend/src/jdtls.rs). The third-party inputs are fetched and
# checksum-verified by scripts/prepare-jdtls.sh, which also prepares the
# tested JDT LS release the extension set is checked against; this script
# only arranges the extensions in the layout the backend expects
# (backend/src/launch.rs), plus the icons and template directories of the New
# Project entries (Java, Spring Boot). The assembled package is read-only at
# runtime; projects are generated in the user's chosen folder.
# Moved from Plugins/mac/Official/JavaSupport/assemble-package.sh in the Lithe
# repository; see MIGRATION.md.

ROOT_DIR="${0:A:h:h}"
PLUGIN_DIR="$ROOT_DIR/io.github.lithe-idea.java-support"
OUTPUT_DIR=""
ARCH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        *) print -u2 -- "Usage: $0 --output package-directory --arch arm64|x86_64|universal"; exit 2 ;;
    esac
done
[[ -n "$OUTPUT_DIR" ]] || { print -u2 -- "--output is required"; exit 2; }
case "$ARCH" in
    arm64) backend_targets=(aarch64-apple-darwin) ;;
    x86_64) backend_targets=(x86_64-apple-darwin) ;;
    universal) backend_targets=(aarch64-apple-darwin x86_64-apple-darwin) ;;
    *) print -u2 -- "Unsupported architecture: $ARCH"; exit 2 ;;
esac

# plugin.json says where the host starts the backend from. The package keeps
# it in bin/, the one directory that may hold an executable.
backend_path=$(/usr/bin/plutil -extract backend.launch.path raw -o - "$PLUGIN_DIR/plugin.json")
[[ "$backend_path" == bin/* && "$backend_path" != *..* ]] || {
    print -u2 -- "plugin.json backend.launch.path must name a file inside bin/, found: $backend_path"
    exit 1
}

jdtls_root=$(LITHE_ARCH="$ARCH" "$ROOT_DIR/scripts/prepare-jdtls.sh")
extensions="$OUTPUT_DIR/jdtls-extensions"
rm -rf "$extensions" "$OUTPUT_DIR/icons" "$OUTPUT_DIR/templates" "$OUTPUT_DIR/l10n" "$OUTPUT_DIR/grammars" "$OUTPUT_DIR/bin"
cp -R "$PLUGIN_DIR/icons" "$OUTPUT_DIR/icons"
cp -R "$PLUGIN_DIR/templates" "$OUTPUT_DIR/templates"
cp -R "$PLUGIN_DIR/l10n" "$OUTPUT_DIR/l10n"
cp -R "$PLUGIN_DIR/grammars" "$OUTPUT_DIR/grammars"
mkdir -p "$extensions"

# Only the extensions are packaged, with their licenses and the pins they
# were verified against. JDT LS itself (plugins, features, config_*) stays out.
for entry in lombok java-debug java-test manifest.json; do
    [[ -e "$jdtls_root/$entry" ]] || {
        print -u2 -- "Prepared JDT LS extensions are missing $entry: $jdtls_root"
        exit 1
    }
    cp -R "$jdtls_root/$entry" "$extensions/$entry"
done
# The plugin's own extension bundle (Eclipse WTP containers, servlet
# container variables) is compiled here against the prepared JDT LS release.
node "$ROOT_DIR/scripts/package/project-model-bundle.mjs" "$jdtls_root" "$extensions" >&2

# The plugin backend is one Mach-O binary: a single architecture is copied as
# built, a universal package joins the two with lipo. Both targets must be
# installed in the Rust toolchain (rustup target add <triple>). The build runs
# from the repository's Cargo workspace; --locked keeps it on the committed
# Cargo.lock, which scripts/verify-sdk-lock.sh checks against sdk.lock.json.
backend_binaries=()
for backend_target in "${backend_targets[@]}"; do
    cargo build --release --locked --manifest-path "$ROOT_DIR/Cargo.toml" \
        -p lithe-java-backend --target "$backend_target" >&2
    backend_binary="$ROOT_DIR/target/$backend_target/release/lithe-java-backend"
    [[ -f "$backend_binary" ]] || {
        print -u2 -- "cargo did not produce the backend binary: $backend_binary"
        exit 1
    }
    backend_binaries+=("$backend_binary")
done
mkdir -p "$OUTPUT_DIR/${backend_path:h}"
if (( ${#backend_binaries} == 1 )); then
    cp "${backend_binaries[1]}" "$OUTPUT_DIR/$backend_path"
else
    /usr/bin/lipo -create "${backend_binaries[@]}" -output "$OUTPUT_DIR/$backend_path"
fi
chmod 755 "$OUTPUT_DIR/$backend_path"

# The backend resolves the language server launch (plugin.json declares
# the function), so the layout it expects is fixed in backend/src/launch.rs
# and checked here, where an extension upgrade that renames or moves a file
# fails instead of at the user's first Java file.
# backend/tests/repository_pins.rs holds this list to the backend's.
for declared in \
    jdtls-extensions/lombok/lombok.jar \
    jdtls-extensions/java-test/runner/com.microsoft.java.test.runner-jar-with-dependencies.jar \
    "$backend_path"; do
    [[ -f "$OUTPUT_DIR/$declared" ]] || {
        print -u2 -- "The backend expects $declared, which the assembled package does not contain. Update scripts/prepare-jdtls.sh or backend/src/launch.rs for the pinned extensions."
        exit 1
    }
done
for directory in jdtls-extensions/java-debug jdtls-extensions/java-test/extensions jdtls-extensions/project-model; do
    [[ -n "$(find "$OUTPUT_DIR/$directory" -maxdepth 1 -name '*.jar' -print -quit 2>/dev/null)" ]] || {
        print -u2 -- "extension bundle directory $directory holds no bundles"
        exit 1
    }
done

# The contract forbids symbolic links inside a package.
if [[ -n "$(find "$OUTPUT_DIR" -type l -print -quit)" ]]; then
    print -u2 -- "The Java Support package contains symbolic links: $(find "$OUTPUT_DIR" -type l -print -quit)"
    exit 1
fi

# Notices are build inputs, copied alongside the binary from locked sources.
node "$ROOT_DIR/scripts/package/collect-notices.mjs" "$OUTPUT_DIR" >&2
