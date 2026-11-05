//! Error, cancellation and response models shared by the Java commands.
//!
//! These are the minimal parts of Rust Core's `protocol` module that the Java
//! domain logic used. They keep the same JSON shapes, so a request that Rust
//! Core answered before the split gets the same answer from the plugin.

pub mod cancellation;
mod contracts;
mod error;

pub use contracts::*;
pub use error::*;
