//! Capabilities the Java rules borrowed from other parts of Rust Core.
//!
//! In Rust Core these functions reached into project indexing, Git and the
//! diagnostics log. A plugin cannot call them, so each one is either
//! reimplemented here with the same observable result, or answers the safe
//! "unknown" value until the host exposes a plugin API for it. Every
//! placeholder is marked `TODO(host-api: API-NN ...)` with its plan number
//! and listed in `MIGRATION.md`.
//!
//! The diagnostics log is the exception: [`diagnostics`] is the single exit
//! through which the domain reports handled failures to the plugin's unified
//! host log, as controlled category codes and counts only.
//!
//! Maven and Gradle settings are not borrowed here. The build-system plugins
//! compute them and hand them to this plugin through a cross-plugin function
//! call (API-35), as described in `lsp::languages::jdt`.

pub mod diagnostics;
pub mod git;
pub mod indexing;
pub mod storage;
