//! The backend driven by a scripted host over in-process pipes.
//!
//! Every wait is bounded, so a broken session fails with the frames it sent
//! instead of hanging.

use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

struct Host {
    input: Option<io::PipeWriter>,
    frames: mpsc::Receiver<Value>,
    exit: mpsc::Receiver<i32>,
}

impl Host {
    fn start() -> Self {
        let backend = lithe_java_backend::backend().expect("functions register");
        let (backend_input, input) = io::pipe().expect("input pipe");
        let (output, backend_output) = io::pipe().expect("output pipe");
        let (exit_sender, exit) = mpsc::channel();
        thread::spawn(move || {
            let _ = exit_sender.send(backend.serve(backend_input, backend_output));
        });
        let (frame_sender, frames) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Some(frame) = read_frame(&mut reader) {
                if frame_sender.send(frame).is_err() {
                    return;
                }
            }
        });
        Self {
            input: Some(input),
            frames,
            exit,
        }
    }

    fn request(&mut self, id: i64, method: &str, params: Value) {
        let body =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        self.input
            .as_mut()
            .expect("open input")
            .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
            .expect("write to the backend");
    }

    /// The response to request `id`, and the `params` of every `lithe/log`
    /// notification the backend sent before it. The backend writes a call's
    /// terminal fact before the call's response, so the facts of a call are
    /// complete once its response arrived.
    fn answer_with_logs(&self, id: i64) -> (Value, Vec<Value>) {
        let mut logs = Vec::new();
        loop {
            let frame = self
                .frames
                .recv_timeout(WAIT)
                .expect("a frame from the backend in time");
            if frame.get("method").and_then(Value::as_str) == Some("lithe/log") {
                logs.push(frame["params"].clone());
            } else if frame["id"] == id && frame.get("method").is_none() {
                return (frame, logs);
            }
        }
    }

    fn answer(&self, id: i64) -> Value {
        self.answer_with_logs(id).0
    }

    fn invoke_with_logs(&mut self, id: i64, function: &str, params: Value) -> (Value, Vec<Value>) {
        self.request(
            id,
            "lithe/function/invoke",
            json!({"functionID": function, "operationID": format!("op-{id}"), "params": params}),
        );
        self.answer_with_logs(id)
    }

    fn invoke(&mut self, id: i64, function: &str, params: Value) -> Value {
        self.invoke_with_logs(id, function, params).0
    }

    fn respond(&mut self, id: Value, result: Value) {
        let body = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
        self.input
            .as_mut()
            .expect("open input")
            .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
            .expect("write response to backend");
    }

    fn notify(&mut self, method: &str, params: Value) {
        let body = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        self.input
            .as_mut()
            .expect("open input")
            .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
            .expect("write notification to backend");
    }

    fn run_until(
        &mut self,
        mut stop: impl FnMut(&Value) -> bool,
        mut on_host_request: impl FnMut(&str, &Value) -> Value,
    ) -> (Value, Vec<Value>) {
        let mut logs = Vec::new();
        loop {
            let frame = self
                .frames
                .recv_timeout(WAIT)
                .expect("a frame from the backend in time");
            if frame.get("method").and_then(Value::as_str) == Some("lithe/log") {
                logs.push(frame["params"].clone());
            } else if let Some(method) = frame.get("method").and_then(Value::as_str) {
                let req_id = frame["id"].clone();
                let answer = on_host_request(method, &frame["params"]);
                let done = stop(&frame);
                self.respond(req_id, answer);
                if done {
                    return (frame, logs);
                }
            } else if stop(&frame) {
                return (frame, logs);
            }
        }
    }

    fn run_until_answer(
        &mut self,
        id: i64,
        on_host_request: impl FnMut(&str, &Value) -> Value,
    ) -> (Value, Vec<Value>) {
        self.run_until(|frame| frame["id"] == id, on_host_request)
    }

    /// Sends `initialize` as a host that offers plugin functions and
    /// `litheLogging` 0, and returns the answer.
    fn initialize(&mut self) -> Value {
        self.request(
            0,
            "initialize",
            json!({"rootUri": "file:///tmp/state/", "capabilities": {
                "experimental": {
                    "litheFunctions": {"versions": [0]},
                    "litheLogging": {"versions": [0]}
                }
            }}),
        );
        self.answer(0)
    }

    fn close(mut self) -> i32 {
        self.input = None;
        self.exit.recv_timeout(WAIT).expect("the backend to end")
    }
}

fn read_frame(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[test]
fn the_backend_offers_the_java_commands_as_plugin_functions() {
    let mut host = Host::start();
    let initialized = host.initialize();
    let encoded = initialized.to_string();
    assert!(encoded.contains("java.className"), "{encoded}");
    assert!(encoded.contains("runConfig.selectJava"), "{encoded}");
    assert_eq!(
        initialized["result"]["capabilities"]["experimental"]["litheLogging"]["version"], 0,
        "{initialized}"
    );

    let class_name = host.invoke(
        1,
        "java.className",
        json!({"source": "package com.example;\nclass App {}", "simpleName": "App"}),
    );
    assert_eq!(
        class_name["result"]["className"], "com.example.App",
        "{class_name}"
    );

    let invalid = host.invoke(2, "java.className", json!({"unexpected": true}));
    assert_eq!(
        invalid["error"]["data"]["code"], "java.invalidRequest",
        "{invalid}"
    );
    assert_eq!(
        invalid["error"]["message"],
        "Invalid Java class name request"
    );

    let session = host.invoke(3, "java.prepareLaunch", json!({}));
    assert_eq!(
        session["error"]["data"]["code"], "java.notSupported",
        "{session}"
    );

    assert_eq!(
        host.close(),
        1,
        "closing stdin without exit is a failure exit"
    );
}

/// The only `lithe/log` fact of `logs` with the given kind.
fn only_fact<'a>(logs: &'a [Value], kind: &str) -> &'a Value {
    let facts: Vec<&Value> = logs.iter().filter(|log| log["kind"] == kind).collect();
    assert_eq!(facts.len(), 1, "expected one {kind} fact in {logs:?}");
    facts[0]
}

#[test]
fn the_backend_reports_restricted_facts_and_suppresses_reads() {
    let mut host = Host::start();
    let initialized = host.initialize();
    assert!(initialized.get("error").is_none(), "{initialized}");

    // A successful editor read is a suppressed fact: no operationFinished.
    let (class_name, class_name_logs) = host.invoke_with_logs(
        1,
        "java.className",
        json!({"source": "package com.example;\nclass App {}", "simpleName": "App"}),
    );
    assert_eq!(
        class_name["result"]["className"], "com.example.App",
        "{class_name}"
    );
    assert!(
        !class_name_logs
            .iter()
            .any(|log| log["kind"] == "operationFinished"),
        "a suppressed success was logged: {class_name_logs:?}"
    );

    // A failure is still logged once, with the error code and no message.
    let (invalid, invalid_logs) =
        host.invoke_with_logs(2, "java.className", json!({"unexpected": true}));
    assert_eq!(
        invalid["error"]["data"]["code"], "java.invalidRequest",
        "{invalid}"
    );
    let invalid_failure = only_fact(&invalid_logs, "operationFailed");
    assert_eq!(invalid_failure["operation"], "java.className");
    assert_eq!(invalid_failure["code"], "java.invalidRequest");
    assert!(
        invalid_failure["durationMs"].is_u64(),
        "durationMs must be an integer: {invalid_failure}"
    );
    // The fact names the operationID the host sent (`op-{id}`).
    assert_eq!(invalid_failure["operationId"], "op-2");

    let (session, session_logs) = host.invoke_with_logs(3, "java.prepareLaunch", json!({}));
    assert_eq!(
        session["error"]["data"]["code"], "java.notSupported",
        "{session}"
    );
    let session_failure = only_fact(&session_logs, "operationFailed");
    assert_eq!(session_failure["operation"], "java.prepareLaunch");
    assert_eq!(session_failure["code"], "java.notSupported");
    assert_eq!(session_failure["operationId"], "op-3");

    // No fact carries an error message, the request's source or a path.
    let all_logs: Vec<Value> = [class_name_logs, invalid_logs, session_logs].concat();
    let encoded = Value::Array(all_logs).to_string();
    assert!(!encoded.contains("\"message\""), "{encoded}");
    assert!(!encoded.contains("package com.example"), "{encoded}");
    assert!(!encoded.contains("/tmp"), "{encoded}");

    assert_eq!(
        host.close(),
        1,
        "closing stdin without exit is a failure exit"
    );
}

#[test]
fn initialize_without_lithe_logging_is_refused() {
    let mut host = Host::start();
    host.request(
        0,
        "initialize",
        json!({"rootUri": "file:///tmp/state/", "capabilities": {
            "experimental": {"litheFunctions": {"versions": [0]}}
        }}),
    );
    let refused = host.answer(0);
    assert!(refused.get("error").is_some(), "{refused}");
    assert!(refused.get("result").is_none(), "{refused}");
    assert_eq!(
        host.close(),
        1,
        "closing stdin without exit is a failure exit"
    );
}

#[test]
fn binary_content_callback_is_available_before_provider_registration() {
    let mut host = Host::start();
    let initialized = host.initialize();
    let exit = host.close();
    assert_eq!(exit, 1);
    assert!(
        initialized.to_string().contains("java.classFileContents"),
        "{initialized}"
    );
}

#[test]
fn navigation_collect_and_invoke_round_trip_with_quiet_collect_logging() {
    let mut host = Host::start();
    let initialized = host.initialize();
    let encoded = initialized.to_string();
    assert!(encoded.contains("java.navigation.collect"), "{encoded}");
    assert!(encoded.contains("java.navigation.invoke"), "{encoded}");

    let (workspace_root, greeter_uri, impl_uri) = if cfg!(windows) {
        (
            r"C:\workspace\nav",
            "file:///C:/workspace/nav/src/Greeter.java",
            "file:///C:/workspace/nav/src/Impl.java",
        )
    } else {
        (
            "/workspace/nav",
            "file:///workspace/nav/src/Greeter.java",
            "file:///workspace/nav/src/Impl.java",
        )
    };
    let context_ref = json!({
        "contextID": "ctx-nav",
        "generation": 1
    });
    let full_context = json!({
        "contextID": "ctx-nav",
        "generation": 1,
        "folders": [{"rootID": "r1", "path": workspace_root}]
    });
    host.notify("initialized", json!({}));
    host.notify(
        "lithe/host/languageServer/didChangeState",
        json!({
            "context": context_ref,
            "serverID": "java",
            "generation": 1,
            "state": "running"
        }),
    );

    // Wait until navigation actions registration occurs (meaning didChangeState processed "running").
    host.run_until(
        |frame| {
            frame.get("method").and_then(Value::as_str) == Some("lithe/host/editor/registerActions")
                && frame.pointer("/params/id").and_then(Value::as_str) == Some("java.navigation")
        },
        |method, _params| match method {
            "lithe/host/workspace/listContexts" => json!({"contexts": [full_context]}),
            "lithe/host/workspace/getContext" => full_context.clone(),
            "lithe/host/editor/registerActions" => json!({"subscriptionID": "sub-nav-1"}),
            "lithe/host/documents/registerContentProvider" => {
                json!({"subscriptionID": "sub-content-1"})
            }
            _ => json!({}),
        },
    );

    let source = "interface Greeter { void greet(); }\nclass Impl implements Greeter {\n    public void greet() {}\n}\n";
    let mut seen_methods = Vec::new();
    host.request(
        10,
        "lithe/function/invoke",
        json!({
            "functionID": "java.navigation.collect",
            "operationID": "op-nav-collect",
            "context": context_ref,
            "params": {
                "document": {"path": "src/App.java"},
                "text": source
            }
        }),
    );
    let (collect_resp, collect_logs) = host.run_until_answer(10, |method, params| {
        seen_methods.push(method.to_owned());
        match method {
            "lithe/host/workspace/listContexts" => json!({"contexts": [full_context]}),
            "lithe/host/workspace/getContext" => full_context.clone(),
            "lithe/host/getCapabilities" => json!({
                "services": {
                    "editor": {
                        "options": {
                            "actionIcons": [
                                "navigate-implemented",
                                "navigate-implementing",
                                "navigate-overridden",
                                "navigate-overriding"
                            ]
                        }
                    }
                }
            }),
            "lithe/host/editor/registerActions" => json!({"subscriptionID": "sub-nav-1"}),
            "lithe/host/languageServer/request" => {
                match params.get("method").and_then(Value::as_str) {
                    Some("textDocument/definition") => json!({
                        "value": [{
                            "uri": greeter_uri,
                            "range": {"start": {"line": 0, "character": 10}, "end": {"line": 0, "character": 17}}
                        }]
                    }),
                    Some("textDocument/implementation") => json!({
                        "value": [{
                            "uri": impl_uri,
                            "range": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 10}}
                        }]
                    }),
                    Some("java/findLinks") => json!({
                        "value": [{
                            "displayName": "Greeter.greet()",
                            "location": {
                                "uri": greeter_uri,
                                "range": {"start": {"line": 0, "character": 25}, "end": {"line": 0, "character": 30}}
                            }
                        }]
                    }),
                    _ => json!({"value": []}),
                }
            }
            _ => json!({}),
        }
    });

    let actions = collect_resp["result"]["actions"]
        .as_array()
        .unwrap_or_else(|| panic!("expected actions array, got {collect_resp}"));
    assert!(
        !actions.is_empty(),
        "expected navigation actions: {collect_resp}"
    );
    assert!(
        actions.iter().any(|a| a["icon"] == "navigate-implemented"),
        "expected navigate-implemented: {actions:?}"
    );
    assert!(
        actions.iter().any(|a| a["icon"] == "navigate-implementing"),
        "expected navigate-implementing: {actions:?}"
    );
    assert!(
        !collect_logs
            .iter()
            .any(|log| log["kind"] == "operationFinished"
                && log["operation"] == "java.navigation.collect"),
        "high-frequency collect must suppress INFO operationFinished: {collect_logs:?}"
    );

    let first_action = actions
        .iter()
        .find(|a| a["icon"] == "navigate-implementing")
        .expect("implementing action");
    host.request(
        11,
        "lithe/function/invoke",
        json!({
            "functionID": "java.navigation.invoke",
            "operationID": "op-nav-invoke",
            "context": context_ref,
            "params": {
                "data": first_action["invoke"]["params"]
            }
        }),
    );
    let (invoke_resp, invoke_logs) = host.run_until_answer(11, |method, params| match method {
        "lithe/host/languageServer/request"
            if params.get("method").and_then(Value::as_str) == Some("textDocument/definition") =>
        {
            json!({
                "value": [{
                    "uri": greeter_uri,
                    "range": {"start": {"line": 0, "character": 10}, "end": {"line": 0, "character": 17}}
                }]
            })
        }
        "lithe/host/languageServer/request" => json!({
            "value": [{
                "displayName": "Greeter.greet()",
                "location": {
                    "uri": greeter_uri,
                    "range": {"start": {"line": 0, "character": 25}, "end": {"line": 0, "character": 30}}
                }
            }]
        }),
        _ => json!({}),
    });

    let locations = invoke_resp["result"]["reveal"]["locations"]
        .as_array()
        .expect("reveal locations");
    assert_eq!(locations.len(), 1, "{invoke_resp}");
    assert_eq!(locations[0]["uri"], greeter_uri);
    assert!(
        invoke_logs
            .iter()
            .any(|log| log["kind"] == "operationFinished"
                && log["operation"] == "java.navigation.invoke"),
        "user click invoke must record operationFinished INFO: {invoke_logs:?}"
    );
    let _ = seen_methods;
    assert_eq!(host.close(), 1);
}
