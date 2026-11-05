//! Plugin backend of the Lithe Java Support plugin.
//!
//! The backend is a [`lithe_plugin_sdk`] process that offers the commands of
//! [`lithe_java_domain::commands`] as plugin functions with the same IDs and
//! JSON payloads Rust Core accepted, and that connects JDT LS to the settings
//! other plugins contribute. The plugin manifest declares it as the plugin's
//! `backend`.
//!
//! The Java rules themselves still run in the host for the editor providers,
//! run configurations and the Java test workflow; the backend is where they
//! move, one host API at a time (see `MIGRATION.md`).
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

pub mod plugin_functions;
pub mod settings_session;

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
pub const KEY_OPERATIONS: &[&str] = &[
    "debug.javaTestLaunch",
    "execution.planLaunchCommand",
    "java.prepareLaunch",
    SETTINGS_CHANGED_FUNCTION_ID,
    "runConfig.selectJava",
];

/// Builds the backend with every Java Support function registered.
///
/// A function can only be suppressed once it is registered, so the successes
/// of the functions outside [`KEY_OPERATIONS`] are suppressed after the
/// registration loop. `languageImport.settingsChanged` is registered after
/// that loop and is a key operation, so it needs no suppression.
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
    backend.function(SETTINGS_CHANGED_FUNCTION_ID, move |params, context| {
        settings_changed(&session, params, context)
    })?;
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

    /// An error code names one cause. The codes `settingsChanged` answers
    /// with, the domain codes and the diagnostics categories of the settings
    /// modules must never share a name.
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
        // The commands, and the one function registered next to them.
        let commands: std::collections::BTreeSet<&str> = SUPPORTED_COMMANDS
            .iter()
            .chain(SESSION_COMMANDS)
            .copied()
            .chain([SETTINGS_CHANGED_FUNCTION_ID])
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
