#!/bin/zsh

set -euo pipefail

# Verifies an assembled Java Support package directory before it is archived:
#   - plugin.json is the repository manifest, unchanged, and declares a
#     plugin without a publisher signature (vendor.signatureRequirement "none");
#   - the package holds only the declared top-level entries and no symbolic
#     links, and nothing outside bin/ is executable;
#   - every JDT LS extension file the backend resolves (backend/src/launch.rs)
#     exists in the layout it expects, and JDT LS itself and a JDK are not
#     packaged: the backend uses the user's own (backend/src/jdtls.rs);
#   - the plugin backend (backend.launch.path in plugin.json) is the only file
#     in bin/, is executable, and contains exactly the requested architectures.

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
    arm64|x86_64) backend_architectures=("$ARCH") ;;
    universal) backend_architectures=(arm64 x86_64) ;;
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

for notice in LICENSE NOTICE; do
    cmp -s "$PACKAGE/$notice" "$ROOT_DIR/$notice" || fail "missing or modified $notice"
done
node "$ROOT_DIR/scripts/package/collect-notices.mjs" --verify "$PACKAGE"

allowed=(plugin.json bin icons templates l10n jdtls-extensions LICENSE NOTICE licenses)
for entry in "$PACKAGE"/*(DN); do
    (( ${allowed[(Ie)${entry:t}]} )) || fail "undeclared top-level entry ${entry:t}"
done
symlink=$(find "$PACKAGE" -type l -print -quit)
[[ -z "$symlink" ]] || fail "symbolic link $symlink"
# bin/ holds the plugin backend; no other directory may carry an executable
# (the backend itself is pinned down below).
outside_bin=$(find "$PACKAGE" -path "$PACKAGE/bin" -prune -o -type f -perm -u+x -print -quit)
[[ -z "$outside_bin" ]] || fail "executable file outside bin/: $outside_bin"

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
[[ "${(j: :)${(o)backend_archs}}" == "${(j: :)${(o)backend_architectures}}" ]] ||
    fail "$backend_path contains ${backend_archs[*]:-no architectures}, expected ${backend_architectures[*]}"

# The language server launch is resolved by the backend (plugin.json declares
# launch.kind "backend" and the function only), so the files are checked
# against the layout fixed in backend/src/launch.rs; repository_pins.rs holds
# this script to that list.
[[ "$(/usr/bin/plutil -extract languageServers.0.launch.kind raw -o - "$manifest")" == "backend" ]] ||
    fail "plugin.json must declare a backend-resolved language server launch"
for declared in \
    jdtls-extensions/lombok/lombok.jar \
    jdtls-extensions/java-test/runner/com.microsoft.java.test.runner-jar-with-dependencies.jar; do
    [[ -f "$PACKAGE/$declared" ]] || fail "the backend expects $declared, which the package does not contain"
done
for directory in jdtls-extensions/java-debug jdtls-extensions/java-test/extensions jdtls-extensions/project-model; do
    [[ -n "$(find "$PACKAGE/$directory" -maxdepth 1 -name '*.jar' -print -quit 2>/dev/null)" ]] ||
        fail "extension bundle directory $directory holds no bundles"
done
for license in lombok/LICENSE-MIT.txt java-debug/LICENSE-EPL-1.0.txt java-test/LICENSE-MIT.txt; do
    [[ -f "$PACKAGE/jdtls-extensions/$license" ]] || fail "missing third-party license jdtls-extensions/$license"
done
# JDT LS and the JDK it runs on are the user's: an Equinox launcher or a Java
# executable in the package means a build copied them in.
bundled=$(find "$PACKAGE" \( \( -type f \( -name 'org.eclipse.equinox.launcher_*.jar' -o -name java -o -name java.exe -o -name 'libjvm.*' \) \) -o \( -type d -name 'config_*' \) \) -print -quit)
[[ -z "$bundled" ]] || fail "the package must not contain JDT LS or a JDK: ${bundled#$PACKAGE/}"

print -u2 -- "Verified Java Support package $PACKAGE ($ARCH)"
