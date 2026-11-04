//! Java domain rules of the Lithe Java Support plugin.
//!
//! This crate holds the Java, JDT LS, Spring and MyBatis logic that used to
//! live in Lithe's Rust Core and macOS host. It has no dependency on Lithe:
//! the few Core types it needs are copied into [`protocol`], [`lsp`] and
//! [`debug`], and every capability that only the host can provide is a
//! `TODO(host-api: API-NN ...)` placeholder, where `API-NN` is the number in
//! the Lithe plugin API plan; the placeholders in [`host`] are the common ones.
//!
//! The plugin backend exposes the logic through [`commands::execute`], which
//! accepts the same command names and JSON payloads Rust Core accepted.

pub mod commands;
pub mod debug;
pub mod execution;
pub mod host;
pub mod jdt_integration;
pub mod languages;
pub mod lsp;
pub mod protocol;
pub mod testing;

#[cfg(test)]
mod tests;
