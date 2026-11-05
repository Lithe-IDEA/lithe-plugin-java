//! Git queries the JDT workspace preparation needs.

use std::path::Path;

/// Returns the entries of `candidates` that the Git repository enclosing
/// `directory` does not track.
///
/// `None` means Git could not answer, and callers must treat every candidate
/// as possibly owned by the user. Until the host offers a read-only Git query
/// to plugins, the plugin always answers `None`, so legacy Eclipse project
/// files are never deleted; JDT LS keeps working with them as before.
// TODO(host-api: API-42 built-in Git read-only): read-only VCS query — "which
// of these paths are untracked" (Rust Core ran `git --literal-pathspecs
// ls-files -z -- <paths>`).
pub fn untracked_candidates(_directory: &Path, _candidates: &[String]) -> Option<Vec<String>> {
    None
}
