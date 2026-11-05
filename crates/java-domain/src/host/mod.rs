//! Capabilities the Java rules borrowed from other parts of Rust Core.
//!
//! In Rust Core these functions reached into project indexing, Git and the
//! diagnostics log. A plugin cannot call them, so each one is either
//! reimplemented here with the same observable result, or answers the safe
//! "unknown" value until the host exposes a plugin API for it. Every
//! placeholder is marked `TODO(host-api: API-NN ...)` with its plan number
//! and listed in `MIGRATION.md`.
//!
//! Maven and Gradle settings are not borrowed here. The build-system plugins
//! compute them and hand them to this plugin through a cross-plugin function
//! call (API-35), as described in `lsp::languages::jdt`.

pub mod git;
pub mod indexing;
pub mod storage;
