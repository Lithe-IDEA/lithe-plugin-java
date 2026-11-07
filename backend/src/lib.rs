//! Plugin backend of the Lithe Java Support plugin.
//!
//! The backend is a [`lithe_plugin_sdk`] process that offers the commands of
//! [`lithe_java_domain::commands`] as plugin functions with the same IDs and
//! JSON payloads Rust Core accepted, and that connects JDT LS to the settings
//! other plugins contribute. The plugin manifest declares it as the plugin's
//! `backend`.
//!
//! The run, debug and test actions of Java files are the backend's own as
//! well ([`run`]): it collects them through editor actions and starts the
//! processes through the host's `run/start` method. What still waits for a
//! host API is listed in `MIGRATION.md`.
//!
//! # Language server launch (serverProcess 3)
//!
//! The manifest declares the `java` language server with
//! `launch.kind: "backend"`, so the host asks [`launch`] for the JDT LS
//! command line, initialization options and startup settings before each
//! start. The launch reads the settings the [`settings_session`] knows for
//! the workspace, so a restart starts with the last pushed settings.
//!
//! # Cross-plugin settings (API-35)
//!
//! Two modules cover the interface `languageImport.settings` and its change
//! function, and they are wired together in [`backend`]:
//!
//! - [`plugin_functions`] asks the Maven and Gradle plugins for the settings
//!   JDT LS imports with. It is the only module that knows the host relay's
//!   wire.
//! - [`settings_session`] keeps the answers per workspace. It collects them
//!   when the host reports that the `java` language server is running, serves
//!   the exported function `languageImport.settingsChanged`, and sends the
//!   merged settings to JDT LS.
//!
//! # Logging
//!
//! The backend reports restricted facts through the SDK's `litheLogging` 1
//! channel; the SDK refuses `initialize` when the host does not offer it. The
//! SDK reports each function's terminal fact by itself: a failure as
//! `operationFailed` with the `pluginErrorCode`, a success as
//! `operationFinished` except for the functions outside [`KEY_OPERATIONS`],
//! whose success is suppressed. Failures the domain layer handled and
//! recovered from are reported as `degraded` through
//! `lithe_java_domain::host::diagnostics`; the settings modules report
//! theirs through the SDK diagnostics directly, with the categories listed in
//! `plugin_functions::RELAY_CATEGORIES` and
//! `settings_session::SESSION_CATEGORIES`. The backend never writes to
//! stderr, and a fact carries no path, source or tool output.

pub mod launch;
pub mod plugin_functions;
pub mod run;
pub mod settings_session;

use crate::launch::LAUNCH_FUNCTION_ID;
use crate::run::{
    RunSession, COLLECT_FUNCTION_ID, RERUN_FUNCTION_ID, RESOLVE_LINK_FUNCTION_ID,
    RUN_ACTION_FUNCTION_ID, RUN_DID_EXIT_NOTIFICATION,
};
use crate::settings_session::{
    HostServices, SettingsSession, LANGUAGE_SERVER_STATE_NOTIFICATION, SETTINGS_CHANGED_FUNCTION_ID,
};
use lithe_java_domain::commands::{self, ExecuteOptions, SESSION_COMMANDS, SUPPORTED_COMMANDS};
use lithe_java_domain::host::diagnostics::{scoped, Reporter};
use lithe_java_domain::protocol::{CoreError, ErrorCode};
use lithe_plugin_sdk::{Backend, Caller, Context, FunctionError, PluginError, SdkError, Value};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Name reported as `serverInfo.name`.
pub const BACKEND_NAME: &str = "lithe-java-backend";

/// Operation name of diagnostics that belong to a host notification and not
/// to a function call. It is one of the SDK's fixed operation names.
const NOTIFICATION_OPERATION: &str = "notification";

/// Functions whose success is a key user operation and is logged at INFO.
/// Every other function is an editor-driven or startup read; its success is
/// suppressed, and its failures are still logged.
///
/// `languageImport.settingsChanged` is a key operation: another plugin
/// reports a change a user made, it happens rarely, and its success is the
/// only trace that the change reached the language server.
/// `java.resolveLaunch` is one too: it runs once per language server start
/// and its success is the only trace of which package files the server
/// started from. `java.runAction` and `java.rerun` are key operations
/// because each is one user click that starts a process; collection and
/// output link resolution run on every edit and every output line, so their
/// successes are suppressed.
pub const KEY_OPERATIONS: &[&str] = &[
    "debug.javaTestLaunch",
    "execution.planLaunchCommand",
    "java.prepareLaunch",
    LAUNCH_FUNCTION_ID,
    RERUN_FUNCTION_ID,
    RUN_ACTION_FUNCTION_ID,
    SETTINGS_CHANGED_FUNCTION_ID,
    "runConfig.selectJava",
];

/// Builds the backend with every Java Support function registered.
///
/// A function can only be suppressed once it is registered, so the successes
/// of the functions outside [`KEY_OPERATIONS`] are suppressed after the
/// registration loop. `languageImport.settingsChanged` and
/// `java.resolveLaunch` are registered after that loop and are key
/// operations, so they need no suppression.
///
/// Every input is a compile-time constant, so the result is the same on each
/// run: it fails only when a function ID is invalid, repeated or over the
/// SDK's limit, or when the notification method is refused, which the test
/// `the_backend_builds_with_every_command_registered` rejects.
pub fn backend() -> Result<Backend, SdkError> {
    let mut backend = Backend::new(BACKEND_NAME).version(env!("CARGO_PKG_VERSION"));
    for command in SUPPORTED_COMMANDS.iter().chain(SESSION_COMMANDS) {
        let command = *command;
        backend.function(command, move |params, context| {
            invoke(command, params, context)
        })?;
    }
    for command in SUPPORTED_COMMANDS.iter().chain(SESSION_COMMANDS) {
        if !KEY_OPERATIONS.contains(command) {
            backend.suppress_success_diagnostics(command)?;
        }
    }

    // The session works outside any function call (a server state
    // notification belongs to none), so it reaches the host through the
    // backend handle.
    let session = Arc::new(SettingsSession::new(Arc::new(HostServices::new(
        backend.handle(),
    ))));
    let handle = backend.handle();
    backend.on_host_notification(LANGUAGE_SERVER_STATE_NOTIFICATION, {
        let session = Arc::clone(&session);
        move |params| {
            // An error from `degraded` only means the arguments were invalid,
            // and the session passes controlled categories and counts.
            let report = |code: &str, count: u64| {
                let _ = handle
                    .diagnostics()
                    .degraded(NOTIFICATION_OPERATION, code, Some(count));
            };
            session.did_change_state(&params, &report);
            Ok(())
        }
    })?;
    backend.function(SETTINGS_CHANGED_FUNCTION_ID, {
        let session = Arc::clone(&session);
        move |params, context| settings_changed(&session, params, context)
    })?;
    backend.function(LAUNCH_FUNCTION_ID, move |params, context| {
        launch::resolve_launch_function(&session, params, context)
    })?;

    // The run session registers the editor actions of every workspace context
    // and keeps the result listener of each test run; both happen outside a
    // function call, so it holds the backend handle too.
    let run_session = Arc::new(RunSession::new(backend.handle()));
    let report: run::Report = {
        let handle = backend.handle();
        Arc::new(move |code: &str, count: u64| {
            let _ = handle
                .diagnostics()
                .degraded(NOTIFICATION_OPERATION, code, Some(count));
        })
    };
    backend.on_initialize({
        let run_session = Arc::clone(&run_session);
        let report = Arc::clone(&report);
        move |_initialize| {
            run_session.sync_after_initialize(Arc::clone(&report));
            Ok(())
        }
    });
    backend.on_contexts_changed({
        let run_session = Arc::clone(&run_session);
        let report = Arc::clone(&report);
        move |_params| {
            run_session.sync_contexts_reporting(&*report);
            Ok(())
        }
    });
    backend.on_host_notification(RUN_DID_EXIT_NOTIFICATION, {
        let run_session = Arc::clone(&run_session);
        move |params| {
            run_session.run_did_exit(&params);
            Ok(())
        }
    })?;
    backend.function(COLLECT_FUNCTION_ID, run::collect_actions_function)?;
    backend.suppress_success_diagnostics(COLLECT_FUNCTION_ID)?;
    backend.function(RUN_ACTION_FUNCTION_ID, {
        let run_session = Arc::clone(&run_session);
        move |params, context| run::run_action_function(&run_session, params, context)
    })?;
    backend.function(RERUN_FUNCTION_ID, {
        let run_session = Arc::clone(&run_session);
        move |params, context| run::rerun_function(&run_session, params, context)
    })?;
    backend.function(RESOLVE_LINK_FUNCTION_ID, run::resolve_output_link_function)?;
    backend.suppress_success_diagnostics(RESOLVE_LINK_FUNCTION_ID)?;
    Ok(backend)
}

/// Runs a call of `languageImport.settingsChanged`.
///
/// The caller and the workspace come from the host's relay, never from the
/// params. The call blocks while the session delivers the new settings to the
/// language server, which is the work the caller asked for. Delivery
/// failures are reported as `degraded` facts of this call and do not fail it.
fn settings_changed(
    session: &SettingsSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let diagnostics = context.diagnostics();
    let operation = context.function_id().to_owned();
    // An error from `degraded` only means the arguments were invalid, so the
    // call's own result stands.
    let report = move |code: &str, count: u64| {
        let _ = diagnostics.degraded(&operation, code, Some(count));
    };
    session
        .settings_changed(
            context.caller().map(Caller::plugin_id),
            context.workspace_context(),
            &params,
            &report,
        )
        .map_err(FunctionError::from)
}

/// Runs one domain command for a plugin function call.
///
/// The call's SDK cancellation is forwarded to the command's cooperative
/// cancellation scope. A call cancelled before the scope existed still
/// answers as cancelled, so a late cancellation is never lost.
fn invoke(command: &str, params: Value, context: &Context) -> Result<Value, FunctionError> {
    static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
    context.check_cancelled()?;
    let operation_id = format!(
        "plugin-call-{}",
        NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    context.cancellation().on_cancel({
        let operation_id = operation_id.clone();
        let cancelled = Arc::clone(&cancelled);
        move || {
            cancelled.store(true, Ordering::SeqCst);
            commands::cancel(&operation_id);
        }
    });
    // The domain layer reports failures it handled itself (for example a
    // storage cleanup that failed) to this thread's reporter. The category is
    // controlled text and the count is a number; an error from `degraded` only
    // means the arguments were invalid, so the call's own result stands.
    let diagnostics = context.diagnostics();
    let operation = context.function_id().to_owned();
    let reporter: Reporter = Rc::new(move |code: &str, count: u64| {
        let _ = diagnostics.degraded(&operation, code, Some(count));
    });
    let result = scoped(reporter, || {
        commands::execute_with_options(
            command,
            params,
            ExecuteOptions {
                operation_id: Some(operation_id),
                timeout_milliseconds: None,
            },
        )
    });
    if cancelled.load(Ordering::SeqCst) {
        return Err(FunctionError::Cancelled);
    }
    result.map_err(function_error)
}

/// Maps a domain failure to the error the host receives.
///
/// The SDK reports the failure once, at the function's result boundary, with
/// the `pluginErrorCode` as its category, so this function logs nothing.
/// `details` may carry paths or tool output and never leaves the backend: it
/// is neither part of the error nor part of any log fact.
fn function_error(error: CoreError) -> FunctionError {
    if error.code == ErrorCode::Cancelled {
        return FunctionError::Cancelled;
    }
    // TODO(host-api: API-37 i18n): the messages are English Core messages.
    // Localize them through the host i18n service once plugin backends can
    // reach it.
    PluginError::new(plugin_error_code(&error.code), error.message).into()
}

/// Stable `pluginErrorCode` for each domain error category.
pub fn plugin_error_code(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidRequest => "java.invalidRequest",
        ErrorCode::WorkspaceNotFound => "java.workspaceNotFound",
        ErrorCode::PermissionDenied => "java.permissionDenied",
        ErrorCode::NotSupported => "java.notSupported",
        ErrorCode::RuntimeMissing => "java.runtimeMissing",
        ErrorCode::ProcessStartFailed => "java.processStartFailed",
        ErrorCode::ProcessFailed => "java.processFailed",
        ErrorCode::ParseFailed => "java.parseFailed",
        ErrorCode::Cancelled => "java.cancelled",
        ErrorCode::TimedOut => "java.timedOut",
        ErrorCode::ConnectionClosed => "java.connectionClosed",
        ErrorCode::Unknown => "java.unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_category_has_a_distinct_valid_plugin_code() {
        let codes = [
            ErrorCode::InvalidRequest,
            ErrorCode::WorkspaceNotFound,
            ErrorCode::PermissionDenied,
            ErrorCode::NotSupported,
            ErrorCode::RuntimeMissing,
            ErrorCode::ProcessStartFailed,
            ErrorCode::ProcessFailed,
            ErrorCode::ParseFailed,
            ErrorCode::Cancelled,
            ErrorCode::TimedOut,
            ErrorCode::ConnectionClosed,
            ErrorCode::Unknown,
        ]
        .map(|code| plugin_error_code(&code));
        let distinct: std::collections::BTreeSet<_> = codes.iter().collect();
        assert_eq!(distinct.len(), codes.len());
        assert!(codes
            .iter()
            .all(|code| lithe_plugin_sdk::is_plugin_error_code(code)));
    }

    /// An error code names one cause. The codes `settingsChanged` and
    /// `java.resolveLaunch` answer with, the domain codes and the diagnostics
    /// categories of the settings and launch modules must never share a name.
    #[test]
    fn settings_relay_codes_never_share_a_name_with_another_code() {
        let domain: Vec<&str> = [
            ErrorCode::InvalidRequest,
            ErrorCode::WorkspaceNotFound,
            ErrorCode::PermissionDenied,
            ErrorCode::NotSupported,
            ErrorCode::RuntimeMissing,
            ErrorCode::ProcessStartFailed,
            ErrorCode::ProcessFailed,
            ErrorCode::ParseFailed,
            ErrorCode::Cancelled,
            ErrorCode::TimedOut,
            ErrorCode::ConnectionClosed,
            ErrorCode::Unknown,
        ]
        .iter()
        .map(plugin_error_code)
        .collect();
        let mut seen: std::collections::BTreeSet<&str> = domain.iter().copied().collect();
        for name in settings_session::CHANGE_ERROR_CODES
            .iter()
            .chain(settings_session::SESSION_CATEGORIES)
            .chain(plugin_functions::RELAY_CATEGORIES)
            .chain(launch::LAUNCH_ERROR_CODES)
            .chain(launch::LAUNCH_CATEGORIES)
            .chain(run::RUN_ERROR_CODES)
            .chain(run::RUN_CATEGORIES)
        {
            assert!(seen.insert(*name), "{name} names two causes");
        }
    }

    /// `main` exits without a message when [`backend`] fails, so this test is
    /// what keeps that branch out of every build that passed CI.
    #[test]
    fn the_backend_builds_with_every_command_registered() {
        for command in SUPPORTED_COMMANDS.iter().chain(SESSION_COMMANDS) {
            assert!(lithe_plugin_sdk::is_function_id(command), "{command}");
        }
        if let Err(error) = backend() {
            panic!("the backend does not build: {}", error.code());
        }
    }

    #[test]
    fn key_operations_are_registered_functions_without_duplicates() {
        // The commands, and the functions registered next to them.
        let commands: std::collections::BTreeSet<&str> = SUPPORTED_COMMANDS
            .iter()
            .chain(SESSION_COMMANDS)
            .copied()
            .chain([
                SETTINGS_CHANGED_FUNCTION_ID,
                LAUNCH_FUNCTION_ID,
                COLLECT_FUNCTION_ID,
                RUN_ACTION_FUNCTION_ID,
                RERUN_FUNCTION_ID,
                RESOLVE_LINK_FUNCTION_ID,
            ])
            .collect();
        let distinct: std::collections::BTreeSet<&str> = KEY_OPERATIONS.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            KEY_OPERATIONS.len(),
            "a key operation is listed twice"
        );
        for operation in KEY_OPERATIONS {
            assert!(
                commands.contains(operation),
                "{operation} is not a registered command"
            );
        }
    }
}
