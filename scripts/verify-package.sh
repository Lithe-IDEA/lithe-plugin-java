#!/bin/zsh

set -euo pipefail

# Verifies an assembled Java Support package directory before it is signed or
# archived:
#   - plugin.json is the repository manifest, unchanged;
#   - the package holds only the declared top-level entries and no symbolic
#     links, and nothing outside runtime/ is executable;
#   - every file plugin.json names for the language server exists;
#   - the packaged JDK is the requested architecture and keeps its upstream
#     code signature (the publisher signature alone does not prove macOS can
#     launch it).
# With --host-tools it also runs Lithe's official package verifier and, when
# the package is signed, checks the publisher signature.

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="dev.lithe.plugin.java-support"
PACKAGE=""
ARCH=""
HOST_TOOLS=""
REQUIRE_SIGNATURE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PACKAGE="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        --host-tools) HOST_TOOLS="$2"; shift 2 ;;
        --require-signature) REQUIRE_SIGNATURE=true; shift ;;
        *) print -u2 -- "Usage: $0 --package directory --arch arm64|x86_64|universal [--host-tools bin-directory] [--require-signature]"; exit 2 ;;
    esac
done
[[ -d "$PACKAGE" ]] || { print -u2 -- "--package must name an assembled package directory"; exit 2; }
case "$ARCH" in
    arm64|x86_64) runtime_architectures=("$ARCH") ;;
    universal) runtime_architectures=(arm64 x86_64) ;;
    *) print -u2 -- "Unsupported architecture: $ARCH"; exit 2 ;;
esac

fail() {
    print -u2 -- "Java Support package check failed: $*"
    exit 1
}

manifest="$PACKAGE/plugin.json"
cmp -s "$manifest" "$ROOT_DIR/$PLUGIN_ID/plugin.json" ||
    fail "plugin.json differs from $PLUGIN_ID/plugin.json"
[[ "$(/usr/bin/plutil -extract id raw -o - "$manifest")" == "$PLUGIN_ID" ]] ||
    fail "plugin.json does not declare $PLUGIN_ID"
[[ "${PACKAGE:A:t}" == "$PLUGIN_ID" ]] ||
    fail "the package directory must be named $PLUGIN_ID"

allowed=(plugin.json icons templates language-server runtime lithe-plugin-signature.json)
for entry in "$PACKAGE"/*(DN); do
    (( ${allowed[(Ie)${entry:t}]} )) || fail "undeclared top-level entry ${entry:t}"
done
symlink=$(find "$PACKAGE" -type l -print -quit)
[[ -z "$symlink" ]] || fail "symbolic link $symlink"
outside_runtime=$(find "$PACKAGE" -path "$PACKAGE/runtime" -prune -o -type f -perm -u+x -print -quit)
[[ -z "$outside_runtime" ]] || fail "executable file outside runtime/: $outside_runtime"

launcher=$(/usr/bin/plutil -extract languageServers.0.launch.entry raw -o - "$manifest")
lombok=$(/usr/bin/plutil -extract languageServers.0.hostIntegration.resources.lombokAgent raw -o - "$manifest")
test_runner=$(/usr/bin/plutil -extract languageServers.0.hostIntegration.resources.testRunner raw -o - "$manifest")
for declared in "$launcher" "$lombok" "$test_runner"; do
    [[ -f "$PACKAGE/$declared" ]] || fail "plugin.json declares $declared, which the package does not contain"
done
bundle_count=$(/usr/bin/plutil -extract languageServers.0.hostIntegration.resources.extensionBundleDirectories raw -o - "$manifest")
for (( index = 0; index < bundle_count; index++ )); do
    directory=$(/usr/bin/plutil -extract "languageServers.0.hostIntegration.resources.extensionBundleDirectories.$index" raw -o - "$manifest")
    [[ -n "$(find "$PACKAGE/$directory" -maxdepth 1 -name '*.jar' -print -quit 2>/dev/null)" ]] ||
        fail "extension bundle directory $directory holds no bundles"
done
for license in LICENSE-EPL-2.0.txt lombok/LICENSE-MIT.txt java-debug/LICENSE-EPL-1.0.txt java-test/LICENSE-MIT.txt; do
    [[ -f "$PACKAGE/language-server/$license" ]] || fail "missing third-party license language-server/$license"
done

present_runtimes=("$PACKAGE"/runtime/*(/N:t))
[[ "${(j: :)${(o)present_runtimes}}" == "${(j: :)${(o)runtime_architectures}}" ]] ||
    fail "runtime/ holds ${present_runtimes[*]:-nothing}, expected ${runtime_architectures[*]}"
for runtime_arch in "${runtime_architectures[@]}"; do
    runtime="$PACKAGE/runtime/$runtime_arch"
    configuration="$PACKAGE/language-server/configuration/$runtime_arch/config.ini"
    [[ -f "$configuration" ]] || fail "missing $configuration"
    [[ -x "$runtime/bin/java" ]] || fail "missing $runtime/bin/java"
    /usr/bin/lipo "$runtime/bin/java" -verify_arch "$runtime_arch" >/dev/null 2>&1 ||
        fail "$runtime/bin/java does not contain $runtime_arch"
    grep -Eq '^JAVA_VERSION="21\.' "$runtime/release" || fail "$runtime is not a Java 21 runtime"
    /usr/bin/codesign --verify --strict "$runtime/bin/java" ||
        fail "$runtime/bin/java lost its upstream code signature"
    /usr/bin/codesign --verify --strict "$runtime/lib/server/libjvm.dylib" ||
        fail "$runtime/lib/server/libjvm.dylib lost its upstream code signature"
done

signature="$PACKAGE/lithe-plugin-signature.json"
if [[ "$REQUIRE_SIGNATURE" == true && ! -f "$signature" ]]; then
    fail "the package is not publisher-signed"
fi
if [[ -n "$HOST_TOOLS" ]]; then
    "$HOST_TOOLS/LitheOfficialPluginVerifier" "$PACKAGE"
    if [[ -f "$signature" ]]; then
        "$HOST_TOOLS/LithePluginPackageSigner" --verify "$PACKAGE"
    fi
fi
print -u2 -- "Verified Java Support package $PACKAGE ($ARCH)"
