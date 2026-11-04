//! Helpers shared by the command-level tests.

use crate::commands::{execute_with_options, ExecuteOptions};
use serde_json::{json, Value};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Runs a Core-style request envelope and returns Core's response envelope.
///
/// Rust Core's `execute_json` accepted `{id, command, payload,
/// timeoutMilliseconds}` and answered `{id, ok, data | error}`. The ported
/// tests keep asserting on that shape, which is also what the plugin backend
/// returns to the host.
pub(super) fn execute_json(request: &str) -> String {
    let request: Value = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => {
            return json!({
                "id": Value::Null,
                "ok": false,
                "error": {"code": "invalid_request", "message": "Invalid request", "details": error.to_string()}
            })
            .to_string()
        }
    };
    let id = request["id"].as_str().map(str::to_owned);
    let command = request["command"].as_str().unwrap_or_default();
    let payload = request.get("payload").cloned().unwrap_or_else(|| json!({}));
    let options = ExecuteOptions {
        operation_id: id.clone(),
        timeout_milliseconds: request["timeoutMilliseconds"].as_u64(),
    };
    let response = match execute_with_options(command, payload, options) {
        Ok(data) => json!({"id": id, "ok": true, "data": data}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    };
    response.to_string()
}

/// A fresh path under the system temporary directory.
///
/// The path combines the process ID, a per-process random nonce and a
/// sequence number, so parallel tests never share cleanup ownership and no
/// test depends on the wall clock.
pub(super) fn temporary_root(label: &str) -> PathBuf {
    static NONCE: OnceLock<u64> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nonce = NONCE.get_or_init(|| RandomState::new().hash_one(std::process::id()));
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lithe-java-{label}-{}-{nonce:x}-{sequence}",
        std::process::id()
    ))
}

#[test]
fn temporary_roots_do_not_alias_with_the_same_label() {
    let roots: std::collections::HashSet<_> =
        (0..10_000).map(|_| temporary_root("same-label")).collect();
    assert_eq!(roots.len(), 10_000, "Each fixture must own a distinct path");
}

#[test]
fn envelopes_report_unknown_commands_as_failures() {
    let response: Value =
        serde_json::from_str(&execute_json(r#"{"id":"x","command":"java.unknown"}"#)).unwrap();
    assert_eq!(response["id"], "x");
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
}
