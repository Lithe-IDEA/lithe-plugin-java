//! Command-level tests ported from Rust Core's `src/tests/` modules.
//!
//! They live inside the crate, as in Core, so they can reach test-only hooks
//! such as [`crate::languages::take_mybatis_disk_reads`].

mod java_commands;
mod java_selection;
mod mybatis;
mod spring;
mod support;
