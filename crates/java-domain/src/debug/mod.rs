//! Java test debug launches.
//!
//! [`DebugLaunchConfiguration`] and [`DebugRequestKind`] are the parts of Rust
//! Core's language-neutral debug contract the Java test launcher produces.

mod java_test;

pub use java_test::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// DAP request used to begin a debuggee session.
pub enum DebugRequestKind {
    /// Start a new debuggee.
    Launch,
    /// Attach to a running debuggee.
    Attach,
}

impl DebugRequestKind {
    /// DAP command name of the request.
    pub fn command(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Attach => "attach",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// Provider-specific launch arguments wrapped in a language-neutral contract.
pub struct DebugLaunchConfiguration {
    /// Name shown for the debug session.
    pub name: String,
    /// Whether the session launches or attaches.
    pub request: DebugRequestKind,
    /// Adapter-specific launch arguments.
    #[serde(default)]
    pub arguments: serde_json::Map<String, Value>,
    /// Optional portable stepping policy projected into adapter-specific launch arguments.
    #[serde(default)]
    pub stepping_filters: Option<DebugSteppingFilters>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Portable class and method filters used for stepping and stack-frame presentation.
pub struct DebugSteppingFilters {
    /// Adapter-neutral class-name patterns; Java also accepts `$JDK` and `$Libraries`.
    #[serde(default)]
    pub class_name_filters: Vec<String>,
    /// Whether compiler-generated methods should be skipped.
    #[serde(default)]
    pub skip_synthetics: bool,
    /// Whether class static initializers should be skipped.
    #[serde(default)]
    pub skip_static_initializers: bool,
    /// Whether constructors should be skipped.
    #[serde(default)]
    pub skip_constructors: bool,
    /// Whether native clients should collapse stack frames matched by the class filters.
    #[serde(default)]
    pub hide_filtered_stack_frames: bool,
}
