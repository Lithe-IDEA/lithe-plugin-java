//! Project indexing exclusions forwarded to JDT LS.

/// Import exclusion globs of the workspace at `root_uri`.
///
/// Rust Core derived them from the project indexing context (Git ignore rules
/// and plugin-declared exclusions). That context belongs to the host.
// TODO(host-api: API-25 language server): indexing exclusions — the host must
// pass the workspace's excluded roots (or the derived globs) to the plugin with
// the launch request.
pub fn java_import_exclusions(_root_uri: &str) -> Vec<String> {
    Vec::new()
}
