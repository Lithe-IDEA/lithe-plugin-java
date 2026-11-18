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
fn launch_reports_the_selected_servers_java_requirement_before_package_checks() {
    use std::fs;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(
        std::env::temp_dir().join(format!("lithe-java-setup-session-{}", std::process::id())),
    );
    fs::create_dir_all(fixture.0.join("plugins")).unwrap();
    fs::create_dir_all(fixture.0.join("config_win")).unwrap();
    fs::write(
        fixture.0.join("plugins/org.eclipse.equinox.launcher_1.jar"),
        b"",
    )
    .unwrap();
    fs::write(
        fixture.0.join("config_win/config.ini"),
        b"osgi.bundles=example\n",
    )
    .unwrap();
    let file = fs::File::create(fixture.0.join("plugins/org.eclipse.jdt.ls.core_1.jar")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "META-INF/MANIFEST.MF",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"Bundle-RequiredExecutionEnvironment: JavaSE-25\n")
        .unwrap();
    zip.finish().unwrap();

    let mut host = Host::start();
    assert!(host.initialize().get("error").is_none());
    let mut params = json!({
        "serverID": "java", "platform": {"os": "windows", "arch": "x86_64"},
        "stateDirectory": fixture.0.join("state"),
        "workspaceStateDirectory": fixture.0.join("workspace-state"),
        "logDirectory": fixture.0.join("log"), "temporaryDirectory": fixture.0.join("tmp"),
        "settings": {"languageServerHome": fixture.0},
        "tools": {
            "jdtls-runtime": {"version": "17.0.19"},
            "java": {"version": "8.0.392", "home": fixture.0.join("project-jdk")}
        }
    });
    let (reply, logs) = host.invoke_with_logs(1, "java.resolveLaunch", params.clone());
    assert_eq!(reply["error"]["data"]["code"], "launch.runtime_too_old");
    let message = reply["error"]["message"].as_str().unwrap();
    assert!(message.contains("Java 25 or later"), "{message}");
    assert!(message.contains("https://adoptium.net/temurin/releases/"));
    assert_eq!(
        only_fact(&logs, "operationFailed")["code"],
        "launch.runtime_too_old"
    );
    assert!(!fixture.0.join("state").exists());

    params["settings"]["languageServerHome"] = json!(fixture.0.join("missing"));
    let (reply, logs) = host.invoke_with_logs(2, "java.resolveLaunch", params);
    assert_eq!(reply["error"]["data"]["code"], "launch.server_invalid");
    let message = reply["error"]["message"].as_str().unwrap();
    assert!(message.contains("explicit choice has been kept"));
    assert!(message.contains("config_win/config.ini"));
    assert!(!message.contains("brew"));
    assert_eq!(
        only_fact(&logs, "operationFailed")["code"],
        "launch.server_invalid"
    );
    assert_eq!(host.close(), 1);
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
