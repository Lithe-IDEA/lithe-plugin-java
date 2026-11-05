//! Plugin backend of the Lithe Java Support plugin.
//!
//! The backend is a [`lithe_plugin_sdk`] process that offers the commands of
//! [`lithe_java_domain::commands`] as plugin functions with the same IDs and
//! JSON payloads Rust Core accepted. It has no host wiring yet: the host
//! still runs these rules itself, and the plugin manifest does not declare a
//! `backend` until the host APIs listed in `MIGRATION.md` exist.
//!
//! TODO(host-api: API-25 language server, API-27 editor actions and markers,
//! API-29 build and test): declare the backend in `plugin.json`
//! (`requires.backend` 2, `backend.launch`, `bin/lithe-java-backend`) once the
//! host can call plugin functions for editor providers, run configurations
//! and the Java test workflow, and move each host call site from Core to
//! these functions.
//!
//! [`plugin_functions`] is the backend's side of the cross-plugin interface
//! `languageImport.settings` (API-35): it asks the Maven and Gradle plugins
//! for the settings JDT LS imports with. It is the only module that knows the
//! host relay's wire.
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
//! `lithe_java_domain::host::diagnostics`. The backend never writes to
//! stderr, and a fact carries no path, source or tool output.

pub mod plugin_functions;

use lithe_java_domain::commands::{self, ExecuteOptions, SESSION_COMMANDS, SUPPORTED_COMMANDS};
use lithe_java_domain::host::diagnostics::{scoped, Reporter};
use lithe_java_domain::protocol::{CoreError, ErrorCode};
use lithe_plugin_sdk::{Backend, Context, FunctionError, PluginError, SdkError, Value};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Name reported as `serverInfo.name`.
pub const BACKEND_NAME: &str = "lithe-java-backend";

/// Functions whose success is a key user operation and is logged at INFO.
/// Every other function is an editor-driven or startup read; its success is
/// suppressed, and its failures are still logged.
pub const KEY_OPERATIONS: &[&str] = &[
    "debug.javaTestLaunch",
    "execution.planLaunchCommand",
    "java.prepareLaunch",
    "runConfig.selectJava",
];

/// Builds the backend with every Java Support function registered.
///
/// A function can only be suppressed once it is registered, so the successes
/// of the functions outside [`KEY_OPERATIONS`] are suppressed after the
/// registration loop.
///
/// Every input is a compile-time constant, so the result is the same on each
/// run: it fails only when a function ID is invalid, repeated or over the
/// SDK's limit, which the test `the_backend_builds_with_every_command_registered`
/// rejects.
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
    Ok(backend)
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
    fn key_operations_are_registered_commands_without_duplicates() {
        let commands: std::collections::BTreeSet<&str> = SUPPORTED_COMMANDS
            .iter()
            .chain(SESSION_COMMANDS)
            .copied()
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
