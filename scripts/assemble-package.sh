#!/bin/zsh

set -euo pipefail

# Assembles the resources of the Java Support plugin package: the pinned
# JDT LS distribution with its Lombok, Debug, and Test extensions, the JDK it
# runs on, and the plugin backend (the Rust binary lithe-java-backend, built
# here for the requested architecture and placed where plugin.json's
# backend.launch.path names it). The third-party inputs are fetched and
# checksum-verified by the shared prepare scripts; this script only arranges
# the result in the layout plugin.json declares, plus the icons and template
# directories of the New Project entries (Java, Spring Boot). The assembled
# package is read-only at runtime; projects are generated in the user's chosen
# folder.
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
    arm64)
        runtime_architectures=("$ARCH")
        backend_targets=(aarch64-apple-darwin)
        ;;
    x86_64)
        runtime_architectures=("$ARCH")
        backend_targets=(x86_64-apple-darwin)
        ;;
    universal)
        runtime_architectures=(arm64 x86_64)
        backend_targets=(aarch64-apple-darwin x86_64-apple-darwin)
        ;;
    *) print -u2 -- "Unsupported architecture: $ARCH"; exit 2 ;;
esac

# plugin.json says where the host starts the backend from. The package keeps
# it in bin/, the one directory (besides runtime/) that may hold an executable.
backend_path=$(/usr/bin/plutil -extract backend.launch.path raw -o - "$PLUGIN_DIR/plugin.json")
[[ "$backend_path" == bin/* && "$backend_path" != *..* ]] || {
    print -u2 -- "plugin.json backend.launch.path must name a file inside bin/, found: $backend_path"
    exit 1
}

jdtls_root=$(LITHE_ARCH="$ARCH" "$ROOT_DIR/scripts/prepare-jdtls.sh")
language_server="$OUTPUT_DIR/language-server"
rm -rf "$language_server" "$OUTPUT_DIR/runtime" "$OUTPUT_DIR/icons" "$OUTPUT_DIR/templates" "$OUTPUT_DIR/bin"
cp -R "$PLUGIN_DIR/icons" "$OUTPUT_DIR/icons"
cp -R "$PLUGIN_DIR/templates" "$OUTPUT_DIR/templates"
mkdir -p "$language_server/configuration"

# Only what a macOS launch reads is packaged. The wrapper scripts and the
# configuration areas of other platforms are left out.
for entry in plugins features lombok java-debug java-test manifest.json LICENSE-EPL-2.0.txt; do
    [[ -e "$jdtls_root/$entry" ]] || {
        print -u2 -- "JDT LS distribution is missing $entry: $jdtls_root"
        exit 1
    }
    cp -R "$jdtls_root/$entry" "$language_server/$entry"
done

# Equinox reads one configuration per architecture. The packaged copy stays
# read-only; the host gives each launch a writable area in its cache.
for runtime_arch in "${runtime_architectures[@]}"; do
    case "$runtime_arch" in
        arm64) configuration_name="config_mac_arm" ;;
        x86_64) configuration_name="config_mac" ;;
    esac
    [[ -f "$jdtls_root/$configuration_name/config.ini" ]] || {
        print -u2 -- "JDT LS configuration is missing: $jdtls_root/$configuration_name/config.ini"
        exit 1
    }
    mkdir -p "$language_server/configuration/$runtime_arch"
    cp "$jdtls_root/$configuration_name/config.ini" "$language_server/configuration/$runtime_arch/config.ini"

    if [[ "$ARCH" == "universal" ]]; then
        override_variable="LITHE_JDK_${runtime_arch:u}_ROOT"
        jdk_root=$(LITHE_JDK_ROOT="${(P)override_variable:-}" LITHE_JDK_TARGET_ARCH="$runtime_arch" \
            "$ROOT_DIR/scripts/prepare-jdk.sh")
    else
        jdk_root=$(LITHE_JDK_TARGET_ARCH="$runtime_arch" "$ROOT_DIR/scripts/prepare-jdk.sh")
    fi
    mkdir -p "$OUTPUT_DIR/runtime"
    cp -R "$jdk_root" "$OUTPUT_DIR/runtime/$runtime_arch"
done

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

# plugin.json names these files exactly, so a JDT LS upgrade that renames one
# fails here instead of at the user's first Java file.
launcher=$(/usr/bin/plutil -extract languageServers.0.launch.entry raw "$PLUGIN_DIR/plugin.json")
test_runner=$(/usr/bin/plutil -extract languageServers.0.hostIntegration.resources.testRunner raw "$PLUGIN_DIR/plugin.json")
for declared in "$launcher" "$test_runner" language-server/lombok/lombok.jar "$backend_path"; do
    [[ -f "$OUTPUT_DIR/$declared" ]] || {
        print -u2 -- "Java Support declares $declared, which the assembled package does not contain. Update io.github.lithe-idea.java-support/plugin.json for the pinned JDT LS version."
        exit 1
    }
done

# The contract forbids symbolic links inside a package.
if [[ -n "$(find "$OUTPUT_DIR" -type l -print -quit)" ]]; then
    print -u2 -- "The Java Support package contains symbolic links: $(find "$OUTPUT_DIR" -type l -print -quit)"
    exit 1
fi
