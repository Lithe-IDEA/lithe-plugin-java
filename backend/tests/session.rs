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

    fn answer(&self, id: i64) -> Value {
        loop {
            let frame = self
                .frames
                .recv_timeout(WAIT)
                .expect("a frame from the backend in time");
            if frame["id"] == id && frame.get("method").is_none() {
                return frame;
            }
        }
    }

    fn invoke(&mut self, id: i64, function: &str, params: Value) -> Value {
        self.request(
            id,
            "lithe/function/invoke",
            json!({"functionID": function, "operationID": format!("op-{id}"), "params": params}),
        );
        self.answer(id)
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
    host.request(
        0,
        "initialize",
        json!({"rootUri": "file:///tmp/state/", "capabilities": {
            "experimental": {"litheFunctions": {"versions": [1]}}
        }}),
    );
    let initialized = host.answer(0);
    let encoded = initialized.to_string();
    assert!(encoded.contains("java.className"), "{encoded}");
    assert!(encoded.contains("runConfig.selectJava"), "{encoded}");

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
