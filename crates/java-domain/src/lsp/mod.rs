//! JDT LS integration rules: workspace keys and policy, launch arguments and
//! settings, readiness, build reports, entry points, tests and navigation.
//!
//! The types defined here are the parts of Rust Core's language-server engine
//! that the Java rules referenced. The engine itself (sessions, JSON-RPC,
//! queues) stays in the host; see `MIGRATION.md`.

pub mod languages;

pub use languages::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Observable lifecycle of one managed language-server session.
pub enum LspLifecycleState {
    /// Session state exists, but the child process has not started.
    Created,
    /// The child process and its standard streams are being created.
    ProcessStarting,
    /// The process is running and the initialize handshake is pending.
    Initializing,
    /// Initialization completed and semantic requests may be sent.
    Ready,
    /// A graceful shutdown is pending or the process is being terminated.
    Stopping,
    /// The session ended normally and will produce no further events.
    Stopped,
    /// Startup, protocol handling, or the child process failed terminally.
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// A JDK installation the platform discovered for the Java language service.
pub struct JavaRuntimeCandidate {
    /// The platform-selected project JDK, independent of the server JVM.
    #[serde(default)]
    pub is_default: bool,
    /// JDK home directory.
    pub home_path: String,
    /// Version reported by `java -version`, such as `25.0.4` or `1.8.0_402`.
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Structured runtime failure with session and protocol-stage context.
pub struct LspRuntimeError {
    /// Stable error code.
    pub code: String,
    /// Language-server provider that failed.
    pub provider_id: String,
    /// Session that failed.
    pub session_id: String,
    /// Lifecycle or protocol phase in which the error occurred.
    pub stage: String,
    /// JSON-RPC method being handled, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Document the request concerned, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_uri: Option<String>,
    /// Human-readable summary.
    pub message: String,
    /// Upstream message, kept verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underlying_message: Option<String>,
    /// Exit code of the language-server process, when it exited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_exit_code: Option<i32>,
    /// Evidence behind an unsuccessful Java launch build. Present only for
    /// `javaBuild` failures; hosts that do not read it are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_build_report: Option<JavaBuildReport>,
    /// Plugin-defined code of a failed plugin function (`error.data.code`),
    /// passed through for the plugin's own UI; Lithe never branches on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_error_code: Option<String>,
}

/// Decodes `%XX` escapes in a URI component, keeping the input when the
/// decoded bytes are not UTF-8.
pub fn percent_decode(value: &str) -> String {
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| value.to_string())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
