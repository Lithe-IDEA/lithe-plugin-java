//! Command-level tests of `runConfig.selectJava`, ported from Rust Core's
//! `src/tests/run_configuration.rs`.
//!
//! Core also read the requirements of a workspace root from its generated
//! run-configuration layer. The plugin cannot read that layer, so the cases
//! run through the requirements snapshot the host passes, and a workspace
//! root is checked to fail clearly instead of silently ignoring requirements.

use super::support::{execute_json, temporary_root};
use serde_json::{json, Value};
use std::fs;

fn select(payload: Value) -> Value {
    serde_json::from_str(&execute_json(
        &json!({"id": "selection", "command": "runConfig.selectJava", "payload": payload})
            .to_string(),
    ))
    .expect("selection response should be JSON")
}

#[test]
fn automatic_java_selection_uses_shared_fixture_and_current_requirements() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-configuration/automatic-java-selection.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let document = json!({"version":1,"toolchains":{"project-jdk":{
            "type":"java", "minimumVersion":case["minimum"]
        }}});
        let expected = &fixture["expected"][case["name"].as_str().unwrap()];
        // Reverse enumeration order too: tie-breaking is not filesystem order.
        for reverse in [false, true] {
            let mut candidates = case["candidates"].as_array().unwrap().clone();
            if reverse {
                candidates.reverse();
            }
            let response = select(json!({
                "requirements": document,
                "candidates": candidates,
                "fallbackId": case["fallbackId"]
            }));
            assert_eq!(response["ok"], true, "{response}");
            assert_eq!(response["data"]["id"], expected["id"], "{}", case["name"]);
            assert_eq!(
                !response["data"]["warning"].is_null(),
                expected["warning"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}

#[test]
fn selection_without_requirements_keeps_the_fallback() {
    let response = select(json!({
        "candidates": [
            {"id":"env","version":"1.8.0_402","priority":0},
            {"id":"jdk17","version":"17.0.10","priority":1}
        ],
        "fallbackId": "env"
    }));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["data"]["id"], "env");
    assert!(response["data"]["warning"].is_null());
}

#[test]
fn a_workspace_root_needs_the_host_requirements_channel() {
    let root = temporary_root("java-selection-root");
    fs::create_dir_all(&root).unwrap();
    let response = select(json!({
        "root": root,
        "candidates": [{"id":"env","version":"17.0.10","priority":0}],
        "fallbackId": "env"
    }));
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "not_supported");
}

#[test]
fn java_selection_rejects_ambiguous_or_invalid_host_snapshots() {
    for payload in [
        json!({"root":"/unused","requirements":{"version":1,"toolchains":{}},"candidates":[]}),
        json!({"requirements":{"version":999,"toolchains":{}},"candidates":[]}),
        json!({"requirements":{"toolchains":{}},"candidates":[]}),
    ] {
        let response = select(payload);
        assert_eq!(response["ok"], false, "{response}");
    }
}
