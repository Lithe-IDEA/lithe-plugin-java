#!/bin/zsh

set -euo pipefail

# Verifies an assembled Java Support package directory before it is archived:
#   - plugin.json is the repository manifest, unchanged, and declares a
#     plugin without a publisher signature (vendor.signatureRequirement "none");
#   - the package holds only the declared top-level entries and no symbolic
#     links, and nothing outside runtime/ and bin/ is executable;
#   - every file plugin.json names for the language server exists;
#   - the plugin backend (backend.launch.path in plugin.json) is the only file
#     in bin/, is executable, and contains exactly the requested architectures;
#   - the packaged JDK is the requested architecture and keeps its upstream
#     code signature (macOS only launches the JDK if it does).

ROOT_DIR="${0:A:h:h}"
PLUGIN_ID="io.github.lithe-idea.java-support"
PACKAGE=""
ARCH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package) PACKAGE="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        *) print -u2 -- "Usage: $0 --package directory --arch arm64|x86_64|universal"; exit 2 ;;
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
[[ "$(/usr/bin/plutil -extract vendor.signatureRequirement raw -o - "$manifest")" == "none" ]] ||
    fail "plugin.json must declare vendor.signatureRequirement none; Lithe Store lists only packages without a publisher signature"
[[ "${PACKAGE:A:t}" == "$PLUGIN_ID" ]] ||
    fail "the package directory must be named $PLUGIN_ID"

allowed=(plugin.json bin icons templates language-server runtime)
for entry in "$PACKAGE"/*(DN); do
    (( ${allowed[(Ie)${entry:t}]} )) || fail "undeclared top-level entry ${entry:t}"
done
symlink=$(find "$PACKAGE" -type l -print -quit)
[[ -z "$symlink" ]] || fail "symbolic link $symlink"
# runtime/ holds the JDK and bin/ holds the plugin backend; no other directory
# may carry an executable (the backend itself is pinned down below).
outside_runtime=$(find "$PACKAGE" \( -path "$PACKAGE/runtime" -o -path "$PACKAGE/bin" \) -prune -o -type f -perm -u+x -print -quit)
[[ -z "$outside_runtime" ]] || fail "executable file outside runtime/ and bin/: $outside_runtime"

backend_path=$(/usr/bin/plutil -extract backend.launch.path raw -o - "$manifest")
[[ "$backend_path" == bin/* && "$backend_path" != *..* ]] ||
    fail "plugin.json backend.launch.path must name a file inside bin/, found: $backend_path"
backend="$PACKAGE/$backend_path"
[[ -f "$backend" && -x "$backend" ]] ||
    fail "plugin.json declares the backend $backend_path, which the package does not contain as an executable file"
packaged_backend_files=("$PACKAGE"/bin/**/*(.DN))
[[ "${#packaged_backend_files}" == 1 && "${packaged_backend_files[1]}" == "$backend" ]] ||
    fail "bin/ must hold only the backend $backend_path, found: ${packaged_backend_files[*]:-nothing}"
backend_archs_text=$(/usr/bin/lipo -archs "$backend" 2>/dev/null) ||
    fail "$backend_path is not a Mach-O binary"
backend_archs=(${=backend_archs_text})
[[ "${(j: :)${(o)backend_archs}}" == "${(j: :)${(o)runtime_architectures}}" ]] ||
    fail "$backend_path contains ${backend_archs[*]:-no architectures}, expected ${runtime_architectures[*]}"

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

print -u2 -- "Verified Java Support package $PACKAGE ($ARCH)"
