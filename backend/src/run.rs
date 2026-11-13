//! Run, debug and test actions of Java files.
//!
//! The host collects the gutter actions of each open `.java` document through
//! editor actions (host method `editor/registerActions`), and the backend
//! starts the chosen target through the host's `run/start` method with the
//! `java` tool the manifest requires. The Java rules (which declarations are
//! runnable, how a test launch is planned, which runner arguments a framework
//! needs) live in [`lithe_java_domain`]; this module only connects them to
//! the host.
//!
//! # Flow
//!
//! 1. After `initialize`, and on each `didChangeContexts`, the
//!    [`RunSession`] lists the workspace contexts and registers the collection
//!    function [`COLLECT_FUNCTION_ID`] for `.java` files in each of them.
//! 2. The host calls [`COLLECT_FUNCTION_ID`] with the document; the backend
//!    asks JDT LS for the main methods and test items of the file and answers
//!    one `Run` action per target, with a `Debug` alternative.
//! 3. A click calls [`RUN_ACTION_FUNCTION_ID`] with the action's params. For
//!    a main class the backend builds the workspace and resolves the classpath
//!    through JDT LS; for a test it plans the launch and asks JDT LS for the
//!    runner arguments. It then calls `run/start`, so the host's Run window
//!    owns the process. `Rerun` comes back through [`RERUN_FUNCTION_ID`] with
//!    the same params.
//! 4. A test launch opens one loopback port the Eclipse JUnit runner reports
//!    to. A listener thread counts the results and shows them in a window
//!    message; it stops when the host reports `run/didExit`.
//!
//! # Not yet covered
//!
//! - Long command lines are passed as they are. The domain's argfile planning
//!   (`execution::launch_command`) is not applied, because macOS accepts the
//!   lengths JDT LS produces for ordinary projects.
//! - The result summary is English text until plugin backends can reach the
//!   host's i18n service (API-37).

use crate::launch::{self, TEST_RUNNER};
use crate::settings_session::{SettingsSession, EXECUTE_COMMAND, REQUEST_METHOD, SERVER_ID};
use lithe_java_domain::debug::java_test_launch;
use lithe_java_domain::lsp::languages::java_main_methods::{
    java_main_methods_command, normalize_java_main_methods,
};
use lithe_java_domain::lsp::languages::java_tests::{
    java_test_items_command, normalize_java_test_items, JavaTestItem as CollectedTestItem,
};
use lithe_java_domain::lsp::languages::{
    java_run_markers, JavaRunMarker, JavaRunMarkerKind, JavaRunMarkersRequest, MainMethodInput,
    RangeInput, TestItemInput,
};
use lithe_java_domain::testing::jdt_test_items::{self, JavaTestItem, LAUNCH_ARGUMENTS_COMMAND};
use lithe_plugin_sdk::{
    BackendHandle, Context, FunctionError, Host, PluginError, SdkError, Subscription, Value,
    WorkspaceContext,
};
use serde_json::{json, Map};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Function the host calls to collect the actions of one `.java` document.
pub const COLLECT_FUNCTION_ID: &str = "java.collectActions";

/// Function the host calls when the user clicks an action or an alternative.
pub const RUN_ACTION_FUNCTION_ID: &str = "java.runAction";

/// Function the host calls to rerun a finished run; its params are the data
/// the start passed as `rerun.data`, which are the action's params.
pub const RERUN_FUNCTION_ID: &str = "java.rerun";

/// Registration ID of the editor actions binding, the same in each context.
pub const ACTIONS_BINDING_ID: &str = "javaRunActions";

/// Host notification that reports a finished run: `{runID, exitCode?, signal?}`.
pub const RUN_DID_EXIT_NOTIFICATION: &str = "lithe/host/run/didExit";

/// Host notification that forwards a notification of a language server the
/// manifest lists under `forwardNotifications`:
/// `{context, serverID, generation, method, params?}`.
pub const LANGUAGE_SERVER_NOTIFICATION: &str = "lithe/host/languageServer/didReceiveNotification";

/// JDT LS notification that reports its service state.
const LANGUAGE_STATUS_METHOD: &str = "language/status";

/// `language/status` type that says the server answers workspace commands.
const SERVICE_READY_TYPE: &str = "ServiceReady";

/// Host method that starts a run owned by the host.
const RUN_START_METHOD: &str = "lithe/host/run/start";

/// Host method that lists the workspace contexts.
const LIST_CONTEXTS_METHOD: &str = "lithe/host/workspace/listContexts";

/// The tool definition of the manifest the run starts; `requiresTools` lists it.
const RUN_TOOL: &str = "java";

/// Debug configuration type of the manifest's `java` debug adapter.
const DEBUG_CONFIGURATION_TYPE: &str = "java";

/// JDT LS command that builds the workspace before a launch.
const BUILD_WORKSPACE_COMMAND: &str = "vscode.java.buildWorkspace";

/// JDT LS command that answers `[modulePaths, classPaths]` of a main class.
const RESOLVE_CLASSPATH_COMMAND: &str = "vscode.java.resolveClasspath";

/// `vscode.java.buildWorkspace` answer that means the build succeeded.
const BUILD_SUCCEEDED: i64 = 1;

/// Timeout of the JDT LS requests a launch makes; a workspace build may take
/// a while on a first run.
const LAUNCH_REQUEST_TIMEOUT_MILLISECONDS: u64 = 120_000;

/// Timeout of the JDT LS requests collection makes; the host bounds the whole
/// collection to eight seconds.
const COLLECT_REQUEST_TIMEOUT_MILLISECONDS: u64 = 6_000;

/// Most actions one answer carries; the contract refuses more.
const MAX_ACTIONS: usize = 1000;

/// How long the result listener waits for the test runner to connect before
/// it gives up on its own; the normal end is the `run/didExit` notification.
const RESULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the result listener checks the stop flag while waiting.
const RESULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How often the startup thread retries the first context sync while the
/// session is not initialized yet, and how many times.
const SYNC_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const SYNC_RETRY_ATTEMPTS: u32 = 40;

/// Plugin error code of a call whose params do not have the documented shape.
pub const RUN_INVALID_PARAMS: &str = "run.invalid_params";

/// Plugin error code of a call without a workspace context.
pub const RUN_CONTEXT_MISSING: &str = "run.context_missing";

/// Plugin error code of a launch whose workspace has no local folder, so the
/// process has no working directory.
pub const RUN_WORKSPACE_UNAVAILABLE: &str = "run.workspace_unavailable";

/// Plugin error code of a launch whose JDT LS request failed or answered an
/// unexpected shape.
pub const RUN_LANGUAGE_SERVER_FAILED: &str = "run.language_server_failed";

/// Plugin error code of a launch whose workspace build failed or was cancelled.
pub const RUN_BUILD_FAILED: &str = "run.build_failed";

/// Plugin error code of a test launch whose file has no matching test.
pub const RUN_NO_TESTS: &str = "run.no_tests";

/// Plugin error code of a launch that could not open a loopback port.
pub const RUN_PORT_UNAVAILABLE: &str = "run.port_unavailable";

/// Plugin error code of a launch the host refused to start.
pub const RUN_HOST_REJECTED: &str = "run.host_rejected";

/// Every plugin error code the run functions answer with.
pub const RUN_ERROR_CODES: &[&str] = &[
    RUN_INVALID_PARAMS,
    RUN_CONTEXT_MISSING,
    RUN_WORKSPACE_UNAVAILABLE,
    RUN_LANGUAGE_SERVER_FAILED,
    RUN_BUILD_FAILED,
    RUN_NO_TESTS,
    RUN_PORT_UNAVAILABLE,
    RUN_HOST_REJECTED,
];

/// Category reported when registering the actions binding in a context failed.
pub const RUN_ACTIONS_REGISTRATION_FAILED: &str = "run.actions_registration_failed";

/// Category reported when the contexts could not be listed, so no binding was
/// registered or refreshed.
pub const RUN_CONTEXTS_UNAVAILABLE: &str = "run.contexts_unavailable";

/// Category reported when the test result summary could not be shown.
pub const RUN_RESULT_SUMMARY_FAILED: &str = "run.result_summary_failed";

/// Reports one degraded category with a count; the backend builds it around
/// its diagnostics channel.
pub type Report = Arc<dyn Fn(&str, u64) + Send + Sync>;

/// Every category this module reports.
pub const RUN_CATEGORIES: &[&str] = &[
    RUN_ACTIONS_REGISTRATION_FAILED,
    RUN_CONTEXTS_UNAVAILABLE,
    RUN_RESULT_SUMMARY_FAILED,
];

/// The run state of the backend: the actions binding per context and the
/// result listener per running test launch. Shared between the functions,
/// the notification handlers and the listener threads.
pub struct RunSession {
    settings: Arc<SettingsSession>,
    handle: BackendHandle,
    registrations: Mutex<HashMap<String, Registration>>,
    listeners: Mutex<HashMap<String, Arc<ResultListener>>>,
}

/// The actions binding registered in one context generation.
struct Registration {
    generation: u64,
    subscription: Subscription,
}

/// What the host sends for an action click or a rerun, as the action's
/// `invoke.params` put it there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    debug: bool,
    kind: TargetKind,
    /// Workspace-relative path of the document.
    path: String,
    main_class: Option<String>,
    project_name: Option<String>,
    test_identifier: Option<String>,
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Main,
    Test,
}

impl Target {
    fn to_value(&self) -> Value {
        json!({
            "mode": if self.debug { "debug" } else { "run" },
            "kind": match self.kind { TargetKind::Main => "main", TargetKind::Test => "test" },
            "path": self.path,
            "mainClass": self.main_class,
            "projectName": self.project_name,
            "testIdentifier": self.test_identifier,
            "label": self.label,
        })
    }

    fn from_value(value: &Value) -> Result<Self, FunctionError> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let debug = match text("mode").as_deref() {
            Some("run") => false,
            Some("debug") => true,
            _ => return Err(failure(RUN_INVALID_PARAMS, "The action names no mode.")),
        };
        let kind = match text("kind").as_deref() {
            Some("main") => TargetKind::Main,
            Some("test") => TargetKind::Test,
            _ => return Err(failure(RUN_INVALID_PARAMS, "The action names no kind.")),
        };
        let path = text("path")
            .filter(|path| !path.is_empty())
            .ok_or_else(|| failure(RUN_INVALID_PARAMS, "The action names no document path."))?;
        let label = text("label")
            .filter(|label| !label.is_empty())
            .ok_or_else(|| failure(RUN_INVALID_PARAMS, "The action names no label."))?;
        Ok(Self {
            debug,
            kind,
            path,
            main_class: text("mainClass"),
            project_name: text("projectName"),
            test_identifier: text("testIdentifier"),
            label,
        })
    }
}

impl RunSession {
    pub fn new(handle: BackendHandle, settings: Arc<SettingsSession>) -> Self {
        Self {
            handle,
            settings,
            registrations: Mutex::new(HashMap::new()),
            listeners: Mutex::new(HashMap::new()),
        }
    }

    /// Lists the contexts and brings the actions bindings in line with them:
    /// a new context or generation gets a binding, a vanished one loses it.
    ///
    /// Answers the number of contexts whose registration failed. An error is
    /// the failed listing itself, before any binding was touched.
    pub fn sync_contexts(&self) -> Result<u64, SdkError> {
        let host = self.handle.host();
        let listed = host.request(LIST_CONTEXTS_METHOD, json!({}))?;
        let contexts = listed
            .get("contexts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut registrations = lock(&self.registrations);
        let mut seen = HashSet::new();
        let mut failures = 0;
        for context in &contexts {
            let Ok(reference) = WorkspaceContext::from_value(context) else {
                continue;
            };
            if !self.settings.server_is_running(&reference) {
                continue;
            }
            seen.insert(reference.context_id().to_owned());
            if let Some(existing) = registrations.get(reference.context_id()) {
                if existing.generation == reference.generation() {
                    continue;
                }
                if let Some(stale) = registrations.remove(reference.context_id()) {
                    // The old generation is gone with its binding; a refused
                    // dispose changes nothing.
                    let _ = stale.subscription.dispose();
                }
            }
            let params = json!({
                "id": ACTIONS_BINDING_ID,
                "function": COLLECT_FUNCTION_ID,
                "selector": {"fileExtensions": ["java"], "fileNames": []},
            });
            match host.editor().register_actions(&reference, params) {
                Ok(subscription) => {
                    registrations.insert(
                        reference.context_id().to_owned(),
                        Registration {
                            generation: reference.generation(),
                            subscription,
                        },
                    );
                }
                Err(_) => failures += 1,
            }
        }
        let gone: Vec<String> = registrations
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(stale) = registrations.remove(&id) {
                let _ = stale.subscription.dispose();
            }
        }
        Ok(failures)
    }

    /// Runs [`Self::sync_contexts`] and reports its failures as `degraded`
    /// facts of the notification operation.
    pub fn sync_contexts_reporting(&self, report: &dyn Fn(&str, u64)) -> bool {
        match self.sync_contexts() {
            Ok(0) => true,
            Ok(failures) => {
                report(RUN_ACTIONS_REGISTRATION_FAILED, failures);
                true
            }
            Err(_) => {
                report(RUN_CONTEXTS_UNAVAILABLE, 1);
                false
            }
        }
    }

    /// Syncs the contexts once the session is initialized.
    ///
    /// The `initialize` hook runs before the answer, and the host refuses
    /// requests until the answer is out, so the first sync runs on its own
    /// thread and retries while the session is not ready. The host also
    /// notifies `didChangeContexts` later; both paths end in the same sync.
    pub fn sync_after_initialize(self: &Arc<Self>, report: Report) {
        let session = Arc::clone(self);
        thread::Builder::new()
            .name("java-run-sync".into())
            .spawn(move || {
                for attempt in 0..SYNC_RETRY_ATTEMPTS {
                    thread::sleep(SYNC_RETRY_INTERVAL);
                    match session.sync_contexts() {
                        Ok(0) => return,
                        Ok(failures) => {
                            report(RUN_ACTIONS_REGISTRATION_FAILED, failures);
                            return;
                        }
                        Err(_) if attempt + 1 < SYNC_RETRY_ATTEMPTS => continue,
                        Err(_) => report(RUN_CONTEXTS_UNAVAILABLE, 1),
                    }
                }
            })
            // A thread that cannot start leaves the bindings to the next
            // `didChangeContexts`; there is nothing else to do about it.
            .ok();
    }

    /// Re-registers the actions binding of a context whose language server
    /// just became ready, so the host collects the actions of the open
    /// documents again.
    ///
    /// The host collects once when a binding is registered and then only on
    /// edits. The first registration runs right after `initialize`, while JDT
    /// LS is still starting, so that collection answers no actions; without
    /// this the gutter stays empty until the user edits the file.
    pub fn did_receive_notification(&self, params: &Value, report: &dyn Fn(&str, u64)) {
        let Some(context_id) = service_ready_context(params) else {
            return;
        };
        if let Some(stale) = lock(&self.registrations).remove(&context_id) {
            let _ = stale.subscription.dispose();
        }
        self.sync_contexts_reporting(report);
    }

    /// Stops the result listener of a finished run.
    pub fn run_did_exit(&self, params: &Value) {
        let Some(run_id) = params.get("runID").and_then(Value::as_str) else {
            return;
        };
        if let Some(listener) = lock(&self.listeners).remove(run_id) {
            listener.stop();
        }
    }
}

/// Builds the actions of one `.java` document for the host.
///
/// Answers no actions when the language server is not running or busy: the
/// gutter is empty until it is, and the host collects again on the next edit.
pub fn collect_actions_function(params: Value, context: &Context) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let reference = workspace_context(context)?;
    let path = params
        .pointer("/document/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| failure(RUN_INVALID_PARAMS, "The document has no path."))?;
    let root = workspace_root(context.host(), reference)?;
    let uri = file_uri(&root.join(path));
    let host = context.host();
    let main_methods = match execute_command(
        host,
        reference,
        java_main_methods_command(&uri),
        COLLECT_REQUEST_TIMEOUT_MILLISECONDS,
    ) {
        Ok(value) => value,
        Err(error) if is_transient(&error) => return Ok(json!({"actions": []})),
        Err(error) => return Err(language_server_failed(error)),
    };
    context.check_cancelled()?;
    let test_items = match execute_command(
        host,
        reference,
        java_test_items_command(&uri),
        COLLECT_REQUEST_TIMEOUT_MILLISECONDS,
    ) {
        Ok(value) => value,
        Err(error) if is_transient(&error) => return Ok(json!({"actions": []})),
        Err(error) => return Err(language_server_failed(error)),
    };
    let methods = normalize_java_main_methods(&main_methods)
        .map(|methods| methods.methods)
        .unwrap_or_default();
    let items = normalize_java_test_items(&test_items)
        .map(|items| items.items)
        .unwrap_or_default();
    let request = JavaRunMarkersRequest {
        main_methods: methods
            .iter()
            .map(|method| MainMethodInput {
                main_class: method.main_class.clone(),
                project_name: method.project_name.clone(),
                range: RangeInput {
                    start_line: method.range.start_line,
                    end_line: Some(method.range.end_line),
                },
            })
            .collect(),
        test_items: items.iter().map(test_item_input).collect(),
        test_cases: Vec::new(),
    };
    let markers = java_run_markers(request)
        .map_err(|error| failure(RUN_INVALID_PARAMS, error.message))?
        .markers;
    let actions: Vec<Value> = markers
        .iter()
        .take(MAX_ACTIONS)
        .enumerate()
        .map(|(index, marker)| action(index, marker, path))
        .collect();
    Ok(json!({"actions": actions}))
}

fn test_item_input(item: &CollectedTestItem) -> TestItemInput {
    TestItemInput {
        id: item.id.clone(),
        label: item.label.clone(),
        full_name: item.full_name.clone(),
        test_level: item.test_level,
        range: item.range.as_ref().map(|range| RangeInput {
            start_line: range.start_line,
            end_line: Some(range.end_line),
        }),
        children: item.children.iter().map(test_item_input).collect(),
    }
}

/// One gutter action: `Run` with a `Debug` alternative, on the first column
/// of the declaration line (one-based, half-open).
fn action(index: usize, marker: &JavaRunMarker, path: &str) -> Value {
    let line = marker.line + 1;
    let (kind, icon) = match marker.kind {
        JavaRunMarkerKind::Main => (TargetKind::Main, "run"),
        JavaRunMarkerKind::TestClass | JavaRunMarkerKind::TestMethod => (TargetKind::Test, "test"),
    };
    let target = |debug: bool| Target {
        debug,
        kind,
        path: path.to_owned(),
        main_class: marker.main_class.clone(),
        project_name: marker.project_name.clone(),
        test_identifier: marker.test_identifier(true),
        label: marker.label.clone(),
    };
    json!({
        "id": format!("a{index}"),
        "range": {
            "start": {"line": line, "column": 1},
            "end": {"line": line, "column": 2},
        },
        "title": format!("Run '{}'", marker.label),
        "icon": icon,
        "invoke": {"functionID": RUN_ACTION_FUNCTION_ID, "params": target(false).to_value()},
        "alternatives": [{
            "id": "debug",
            "title": format!("Debug '{}'", marker.label),
            "icon": "debug",
            "invoke": {"functionID": RUN_ACTION_FUNCTION_ID, "params": target(true).to_value()},
        }],
    })
}

/// Runs a click on an action: the host passes the action's params as `data`.
pub fn run_action_function(
    session: &RunSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    let data = params
        .get("data")
        .filter(|data| data.is_object())
        .ok_or_else(|| failure(RUN_INVALID_PARAMS, "The click carries no action data."))?;
    start(session, &Target::from_value(data)?, context)
}

/// Runs a rerun: the host passes the start's `rerun.data` as the params.
pub fn rerun_function(
    session: &RunSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    start(session, &Target::from_value(&params)?, context)
}

/// Prepares the process of a target and asks the host to start it.
fn start(session: &RunSession, target: &Target, context: &Context) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let reference = workspace_context(context)?;
    let host = context.host();
    let root = workspace_root(host, reference)?;
    let debug_port = if target.debug {
        Some(free_port()?)
    } else {
        None
    };
    let (mut process, listener) = match target.kind {
        TargetKind::Main => (main_process(host, reference, &root, target)?, None),
        TargetKind::Test => {
            let (process, listener) = test_process(session, host, reference, &root, target)?;
            (process, Some(listener))
        }
    };
    context.check_cancelled()?;
    if let Some(port) = debug_port {
        // The agent goes first so it applies before any other VM argument;
        // Spring DevTools restarts would detach the debugger, so they are off.
        let mut arguments = vec![
            format!(
                "-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=localhost:{port}"
            ),
            "-Dspring.devtools.restart.enabled=false".to_owned(),
        ];
        arguments.append(&mut process.arguments);
        process.arguments = arguments;
    }
    let mut params = json!({
        "context": reference.to_value(),
        "label": target.label,
        "process": {
            "executable": {"kind": "tool", "tool": RUN_TOOL},
            "arguments": process.arguments,
            "workingDirectory": process.working_directory,
            "environment": {},
        },
        "rerun": {"function": RERUN_FUNCTION_ID, "data": target.to_value()},
    });
    if let Some(port) = debug_port {
        params["debug"] = json!({
            "configurationType": DEBUG_CONFIGURATION_TYPE,
            "target": {"kind": "connection", "port": port},
        });
    }
    let started = match host.request(RUN_START_METHOD, params) {
        Ok(started) => started,
        Err(error) => {
            if let Some(listener) = listener {
                listener.stop();
            }
            return Err(failure(RUN_HOST_REJECTED, error.message().to_owned()));
        }
    };
    // Without a run ID no `didExit` can reach the listener; it then ends by
    // its own timeout after one connection or none.
    if let (Some(listener), Some(run_id)) = (listener, started.get("runID").and_then(Value::as_str))
    {
        lock(&session.listeners).insert(run_id.to_owned(), listener);
    }
    Ok(Value::Null)
}

// Refuse launching stale bytecode when JDT reports compiler errors.
fn require_successful_build(build: &Value) -> Result<(), FunctionError> {
    match build.as_i64() {
        Some(BUILD_SUCCEEDED) => {}
        Some(_) => {
            return Err(failure(
                RUN_BUILD_FAILED,
                "The workspace build failed or was cancelled.",
            ))
        }
        None => {
            return Err(failure(
                RUN_LANGUAGE_SERVER_FAILED,
                "The build answered without a result code.",
            ))
        }
    }
    Ok(())
}

/// The `java` arguments and working directory of one launch.
struct Process {
    arguments: Vec<String>,
    /// Workspace-relative; `.` is the first folder itself.
    working_directory: String,
}

/// Builds the workspace and resolves the classpath of a main class.
fn main_process(
    host: &Host,
    reference: &WorkspaceContext,
    root: &Path,
    target: &Target,
) -> Result<Process, FunctionError> {
    let main_class = target
        .main_class
        .as_deref()
        .filter(|class| !class.is_empty())
        .ok_or_else(|| failure(RUN_INVALID_PARAMS, "The action names no main class."))?;
    let mut build_request = json!({
        "mainClass": main_class,
        "filePath": root.join(&target.path).to_string_lossy(),
        "isFullBuild": false,
    });
    if let Some(project) = &target.project_name {
        build_request["projectName"] = json!(project);
    }
    let build = execute_command(
        host,
        reference,
        json!({"command": BUILD_WORKSPACE_COMMAND, "arguments": [build_request.to_string()]}),
        LAUNCH_REQUEST_TIMEOUT_MILLISECONDS,
    )
    .map_err(language_server_failed)?;
    require_successful_build(&build)?;
    let classpath = execute_command(
        host,
        reference,
        json!({
            "command": RESOLVE_CLASSPATH_COMMAND,
            "arguments": [main_class, target.project_name.clone().unwrap_or_default()],
        }),
        LAUNCH_REQUEST_TIMEOUT_MILLISECONDS,
    )
    .map_err(language_server_failed)?;
    let paths = |index: usize| -> Vec<String> {
        classpath
            .get(index)
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let module_paths = paths(0);
    let class_paths = paths(1);
    if module_paths.is_empty() && class_paths.is_empty() {
        return Err(failure(
            RUN_LANGUAGE_SERVER_FAILED,
            "The classpath of the main class could not be resolved.",
        ));
    }
    let mut arguments = Vec::new();
    if !module_paths.is_empty() {
        arguments.push("--module-path".to_owned());
        arguments.push(module_paths.join(PATH_SEPARATOR));
    }
    if !class_paths.is_empty() {
        arguments.push("-cp".to_owned());
        arguments.push(class_paths.join(PATH_SEPARATOR));
    }
    arguments.push(main_class.to_owned());
    Ok(Process {
        arguments,
        working_directory: ".".to_owned(),
    })
}

/// Plans a test launch with JDT LS and opens the result port the runner
/// reports to. The listener runs until the host reports the run's exit.
fn test_process(
    session: &RunSession,
    host: &Host,
    reference: &WorkspaceContext,
    root: &Path,
    target: &Target,
) -> Result<(Process, Arc<ResultListener>), FunctionError> {
    let file_path = root.join(&target.path);
    let file_name = file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let found = execute_command(
        host,
        reference,
        java_test_items_command(&file_uri(&file_path)),
        LAUNCH_REQUEST_TIMEOUT_MILLISECONDS,
    )
    .map_err(language_server_failed)?;
    let items: Vec<JavaTestItem> = match found {
        Value::Null => Vec::new(),
        value => serde_json::from_value(value).map_err(|_| {
            failure(
                RUN_LANGUAGE_SERVER_FAILED,
                "The test items answered an unexpected shape.",
            )
        })?,
    };
    let plan =
        jdt_test_items::plan_debug_launch(&items, &file_name, target.test_identifier.as_deref())
            .map_err(|error| failure(RUN_NO_TESTS, error.message))?;
    let launch = execute_command(
        host,
        reference,
        json!({"command": LAUNCH_ARGUMENTS_COMMAND, "arguments": [plan.launch_arguments_request()]}),
        LAUNCH_REQUEST_TIMEOUT_MILLISECONDS,
    )
    .map_err(language_server_failed)?;
    let launch = jdt_test_items::parse_launch_arguments(&launch)
        .map_err(|error| failure(RUN_LANGUAGE_SERVER_FAILED, error.message))?;
    let testng_runner = testng_runner_path();
    let debug_target = jdt_test_items::debug_launch_target(
        &file_path.to_string_lossy(),
        &file_name,
        plan,
        launch,
        testng_runner.as_deref(),
    )
    .map_err(|error| failure(RUN_LANGUAGE_SERVER_FAILED, error.message))?;
    let working_directory = relative_working_directory(root, &debug_target.working_directory);
    let listener = ResultListener::start(
        session.handle.host(),
        reference.clone(),
        target.label.clone(),
    )?;
    let configuration = java_test_launch(debug_target.launch_request(listener.port))
        .map_err(|error| failure(RUN_INVALID_PARAMS, error.message))?;
    let arguments = launch_arguments(&configuration.arguments);
    Ok((
        Process {
            arguments,
            working_directory,
        },
        listener,
    ))
}

/// Turns the DAP-style launch arguments the domain produces into `java`
/// arguments: VM arguments, module path, classpath, main class, program
/// arguments.
fn launch_arguments(configuration: &Map<String, Value>) -> Vec<String> {
    let strings = |key: &str| -> Vec<String> {
        configuration
            .get(key)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut arguments = strings("vmArgs");
    let module_paths = strings("modulePaths");
    if !module_paths.is_empty() {
        arguments.push("--module-path".to_owned());
        arguments.push(module_paths.join(PATH_SEPARATOR));
    }
    let class_paths = strings("classPaths");
    if !class_paths.is_empty() {
        arguments.push("-cp".to_owned());
        arguments.push(class_paths.join(PATH_SEPARATOR));
    }
    if let Some(main_class) = configuration.get("mainClass").and_then(Value::as_str) {
        arguments.push(main_class.to_owned());
    }
    arguments.extend(strings("args"));
    arguments
}

/// The TestNG runner shipped in the package, when it is there.
fn testng_runner_path() -> Option<String> {
    let path = launch::package_root().ok()?.join(TEST_RUNNER);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

/// Maps the absolute working directory JDT LS answers to the
/// workspace-relative form `run/start` accepts; a directory outside the
/// first folder falls back to the folder itself.
fn relative_working_directory(root: &Path, working_directory: &str) -> String {
    let directory = Path::new(working_directory);
    match directory.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => ".".to_owned(),
    }
}

/// Counts the results the Eclipse JUnit runner streams to the result port
/// and shows them when the runner disconnects.
struct ResultListener {
    port: u16,
    stop: Arc<AtomicBool>,
}

/// Counts of one test run as the Eclipse runner reports them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ResultCounts {
    started: u64,
    failed: u64,
    errors: u64,
    ignored: u64,
}

impl ResultCounts {
    /// Applies one line of the runner's protocol: `%TESTS` starts a test,
    /// `%FAILED` and `%ERROR` record its outcome. An ignored test starts with
    /// `@Ignore:` in its name.
    fn apply(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("%TESTS") {
            self.started += 1;
            if rest.contains("@Ignore:") || rest.contains("@AssumptionFailure:") {
                self.ignored += 1;
            }
        } else if line.starts_with("%FAILED") {
            self.failed += 1;
        } else if line.starts_with("%ERROR") {
            self.errors += 1;
        }
    }

    fn passed(&self) -> u64 {
        self.started
            .saturating_sub(self.failed)
            .saturating_sub(self.errors)
            .saturating_sub(self.ignored)
    }

    fn summary(&self, name: &str) -> String {
        let mut parts = vec![format!("{} passed", self.passed())];
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        if self.errors > 0 {
            parts.push(format!("{} errors", self.errors));
        }
        if self.ignored > 0 {
            parts.push(format!("{} skipped", self.ignored));
        }
        format!("{name}: {}", parts.join(", "))
    }

    fn is_success(&self) -> bool {
        self.failed == 0 && self.errors == 0
    }
}

impl ResultListener {
    fn start(
        host: Host,
        reference: WorkspaceContext,
        name: String,
    ) -> Result<Arc<Self>, FunctionError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
            .map_err(|error| failure(RUN_PORT_UNAVAILABLE, error.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|error| failure(RUN_PORT_UNAVAILABLE, error.to_string()))?
            .port();
        let stop = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Self {
            port,
            stop: Arc::clone(&stop),
        });
        thread::Builder::new()
            .name(format!("java-test-results-{port}"))
            .spawn(move || serve(listener, stop, host, reference, name))
            .map_err(|error| failure(RUN_PORT_UNAVAILABLE, error.to_string()))?;
        Ok(result)
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Waits for the runner, reads its protocol until it disconnects or the run
/// stops, then shows the summary.
fn serve(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    host: Host,
    reference: WorkspaceContext,
    name: String,
) {
    let started = Instant::now();
    let stream = loop {
        if stop.load(Ordering::SeqCst) || started.elapsed() > RESULT_CONNECTION_TIMEOUT {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(RESULT_POLL_INTERVAL);
            }
            Err(_) => return,
        }
    };
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(RESULT_POLL_INTERVAL)).is_err()
    {
        return;
    }
    let mut counts = ResultCounts::default();
    let mut saw_protocol = false;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.starts_with('%') {
                    saw_protocol = true;
                    counts.apply(line.trim_end());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    // The TestNG runner speaks another protocol; its results are in the
    // run's output, so only the end is reported.
    let text = if saw_protocol {
        counts.summary(&name)
    } else {
        format!("{name}: finished")
    };
    let params = json!({"text": text});
    let shown = if saw_protocol && !counts.is_success() {
        host.window().show_warning(&reference, params)
    } else {
        host.window().show_information(&reference, params)
    };
    if shown.is_err() {
        // The message is the only thing that failed; the counts were seen.
        let _ = host_diagnostics_degraded(&host, RUN_RESULT_SUMMARY_FAILED);
    }
}

/// Reports a category of the notification operation through the host
/// connection the listener holds. The listener has no backend handle; the
/// report goes through the SDK's own degraded channel when it is reachable.
fn host_diagnostics_degraded(_host: &Host, _code: &str) -> Result<(), SdkError> {
    // TODO(sdk): `Host` exposes no diagnostics; the summary failure is lost
    // until the SDK offers them off the backend handle on a thread.
    Ok(())
}

/// Runs one JDT LS command through the host and answers its `result`.
fn execute_command(
    host: &Host,
    reference: &WorkspaceContext,
    command: Value,
    timeout_milliseconds: u64,
) -> Result<Value, SdkError> {
    let answer = host.request(
        REQUEST_METHOD,
        json!({
            "context": reference.to_value(),
            "serverID": SERVER_ID,
            "method": EXECUTE_COMMAND,
            "params": command,
            "timeoutMilliseconds": timeout_milliseconds,
        }),
    )?;
    Ok(answer.get("value").cloned().unwrap_or(Value::Null))
}

/// A host failure collection should answer with no actions rather than fail:
/// the server is not running yet, is busy, or the request timed out.
fn is_transient(error: &SdkError) -> bool {
    matches!(
        error.code(),
        "plugin.host.languageServerUnavailable"
            | "plugin.host.busy"
            | "plugin.host.timeout"
            | "plugin.host.cancelled"
            | "plugin.host.languageServerError"
    )
}

fn language_server_failed(error: SdkError) -> FunctionError {
    failure(RUN_LANGUAGE_SERVER_FAILED, error.message().to_owned())
}

fn workspace_context(context: &Context) -> Result<&WorkspaceContext, FunctionError> {
    context
        .workspace_context()
        .ok_or_else(|| failure(RUN_CONTEXT_MISSING, "The call names no workspace."))
}

/// The local path of the workspace's first folder.
fn workspace_root(host: &Host, reference: &WorkspaceContext) -> Result<PathBuf, FunctionError> {
    let context = host.workspace().get_context(reference)?;
    context
        .pointer("/folders/0/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            failure(
                RUN_WORKSPACE_UNAVAILABLE,
                "The workspace has no local folder.",
            )
        })
}

/// A free loopback port for the debug agent; the port is released before
/// the JVM binds it, which is the usual race every debugger accepts.
fn free_port() -> Result<u16, FunctionError> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| failure(RUN_PORT_UNAVAILABLE, error.to_string()))
}

/// `file:` URI of an absolute path, percent-encoding what RFC 3986 reserves.
pub(crate) fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut uri = String::from("file://");
    for byte in text.bytes() {
        let keep = byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte);
        if keep {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// Path separator of the `-cp` and `--module-path` values.
const PATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

fn failure(code: &'static str, message: impl Into<String>) -> FunctionError {
    PluginError::new(code, message).into()
}

/// The context ID of a forwarded `language/status` `ServiceReady` of the
/// `java` server, `None` for every other notification.
fn service_ready_context(params: &Value) -> Option<String> {
    if params.get("serverID").and_then(Value::as_str) != Some(SERVER_ID)
        || params.get("method").and_then(Value::as_str) != Some(LANGUAGE_STATUS_METHOD)
        || params
            .get("params")
            .and_then(|status| status.get("type"))
            .and_then(Value::as_str)
            != Some(SERVICE_READY_TYPE)
    {
        return None;
    }
    WorkspaceContext::from_value(&params["context"])
        .ok()
        .map(|reference| reference.context_id().to_owned())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lithe_java_domain::lsp::languages::JavaRunMarkerStatus;

    #[test]
    fn compiler_errors_do_not_launch_previous_bytecode() {
        assert!(require_successful_build(&json!(BUILD_SUCCEEDED)).is_ok());
        assert!(require_successful_build(&json!(2)).is_err());
        assert!(require_successful_build(&Value::Null).is_err());
    }

    #[test]
    fn only_a_service_ready_status_of_the_java_server_names_its_context() {
        let ready = json!({
            "context": {"contextID": "ctx-1", "generation": 3},
            "serverID": SERVER_ID,
            "generation": 2,
            "method": LANGUAGE_STATUS_METHOD,
            "params": {"type": SERVICE_READY_TYPE, "message": "ServiceReady"},
        });
        assert_eq!(service_ready_context(&ready).as_deref(), Some("ctx-1"));
        let mut other_server = ready.clone();
        other_server["serverID"] = json!("kotlin");
        let mut other_method = ready.clone();
        other_method["method"] = json!("language/progressReport");
        let mut starting = ready.clone();
        starting["params"]["type"] = json!("Starting");
        let mut no_params = ready.clone();
        no_params.as_object_mut().unwrap().remove("params");
        let mut no_context = ready.clone();
        no_context.as_object_mut().unwrap().remove("context");
        for value in [other_server, other_method, starting, no_params, no_context] {
            assert_eq!(service_ready_context(&value), None, "{value}");
        }
    }

    #[test]
    fn a_target_round_trips_through_its_params() {
        let target = Target {
            debug: true,
            kind: TargetKind::Test,
            path: "src/test/java/OrderTest.java".into(),
            main_class: None,
            project_name: Some("shop".into()),
            test_identifier: Some("item-1".into()),
            label: "OrderTest".into(),
        };
        assert_eq!(Target::from_value(&target.to_value()).unwrap(), target);
    }

    #[test]
    fn a_target_without_mode_or_path_is_invalid() {
        for value in [
            json!({"kind": "main", "path": "A.java", "label": "A"}),
            json!({"mode": "run", "kind": "main", "label": "A"}),
            json!({"mode": "step", "kind": "main", "path": "A.java", "label": "A"}),
        ] {
            match Target::from_value(&value) {
                Err(FunctionError::Plugin(error)) => assert_eq!(error.code(), RUN_INVALID_PARAMS),
                other => panic!("{value} was accepted: {other:?}"),
            }
        }
    }

    #[test]
    fn every_marker_becomes_a_run_action_with_a_debug_alternative() {
        let marker = JavaRunMarker {
            line: 4,
            end_line: 9,
            kind: JavaRunMarkerKind::Main,
            label: "App.main()".into(),
            main_class: Some("com.example.App".into()),
            project_name: Some("app".into()),
            test_class: None,
            test_method: None,
            test_item_id: None,
            status: JavaRunMarkerStatus::None,
        };
        let action = action(0, &marker, "src/main/java/com/example/App.java");
        assert_eq!(action["id"], "a0");
        assert_eq!(action["range"]["start"]["line"], 5);
        assert_eq!(action["icon"], "run");
        assert_eq!(action["invoke"]["functionID"], RUN_ACTION_FUNCTION_ID);
        assert_eq!(action["invoke"]["params"]["mode"], "run");
        assert_eq!(action["invoke"]["params"]["mainClass"], "com.example.App");
        assert_eq!(action["alternatives"][0]["icon"], "debug");
        assert_eq!(
            action["alternatives"][0]["invoke"]["params"]["mode"],
            "debug"
        );
        assert_eq!(
            action["alternatives"][0]["invoke"]["params"]["kind"],
            "main"
        );
    }

    #[test]
    fn launch_arguments_follow_the_java_command_line_order() {
        let configuration = json!({
            "vmArgs": ["-ea"],
            "modulePaths": [],
            "classPaths": ["/a.jar", "/classes"],
            "mainClass": "org.eclipse.jdt.internal.junit.runner.RemoteTestRunner",
            "args": ["-port", "5000"],
        });
        let arguments = launch_arguments(configuration.as_object().unwrap());
        let expected_classpath = format!("/a.jar{PATH_SEPARATOR}/classes");
        assert_eq!(
            arguments,
            vec![
                "-ea",
                "-cp",
                expected_classpath.as_str(),
                "org.eclipse.jdt.internal.junit.runner.RemoteTestRunner",
                "-port",
                "5000",
            ]
        );
    }

    #[test]
    fn result_counts_follow_the_eclipse_runner_protocol() {
        let mut counts = ResultCounts::default();
        for line in [
            "%TESTC  3 v2",
            "%TESTS  1,creates(OrderTest)",
            "%TESTE  1,creates(OrderTest)",
            "%TESTS  2,fails(OrderTest)",
            "%FAILED 2,fails(OrderTest)",
            "%TESTE  2,fails(OrderTest)",
            "%TESTS  3,@Ignore: skipped(OrderTest)",
            "%TESTE  3,@Ignore: skipped(OrderTest)",
            "%RUNTIME120",
        ] {
            counts.apply(line);
        }
        assert_eq!(counts.passed(), 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.ignored, 1);
        assert!(!counts.is_success());
        assert_eq!(
            counts.summary("OrderTest"),
            "OrderTest: 1 passed, 1 failed, 1 skipped"
        );
    }

    #[test]
    fn working_directories_are_relative_to_the_first_folder() {
        let root = Path::new("/work/shop");
        assert_eq!(relative_working_directory(root, "/work/shop"), ".");
        assert_eq!(
            relative_working_directory(root, "/work/shop/orders"),
            "orders"
        );
        assert_eq!(relative_working_directory(root, "/elsewhere"), ".");
    }

    #[test]
    fn file_uris_percent_encode_reserved_characters() {
        assert_eq!(
            file_uri(Path::new("/Users/me/My Project/src/Main.java")),
            "file:///Users/me/My%20Project/src/Main.java"
        );
    }

    #[test]
    fn run_codes_are_valid_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for code in RUN_ERROR_CODES.iter().chain(RUN_CATEGORIES) {
            assert!(lithe_plugin_sdk::is_plugin_error_code(code), "{code}");
            assert!(seen.insert(*code), "{code} is listed twice");
        }
    }
}
