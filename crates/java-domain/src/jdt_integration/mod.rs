//! Rules of the `jdt` host integration that the macOS host used to own.
//!
//! The host located JDT LS and its JDK through the plugin declaration, kept
//! the per-workspace JDT LS indexes in its cache and removed them when the
//! plugin went away. These modules are the decision rules of that code,
//! rewritten without any host type:
//!
//! - [`launch_resources`] lists the extension bundles and names missing
//!   files for the JDT LS launch the backend resolves (`backend/src/launch.rs`
//!   fixes the package layout);
//! - [`runtime`] validates the packaged JDK and decides when cached state may
//!   be discarded;
//! - [`workspace_state`] fingerprints a workspace, resets its index and
//!   applies the cache-retention policy.
//!
//! TODO(host-api: API-25 language server): the host still drives these rules
//! through its generic language-server integration until a plugin backend can
//! answer `languageServer.prepareLaunch`, `languageServer.workspaceFingerprint`
//! and `languageServer.resetWorkspaceState` itself. See `MIGRATION.md`.

pub mod launch_resources;
pub mod runtime;
pub mod workspace_state;
