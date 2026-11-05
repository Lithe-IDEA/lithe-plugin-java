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

pub mod plugin_functions;

use lithe_java_domain::commands::{self, ExecuteOptions, SESSION_COMMANDS, SUPPORTED_COMMANDS};
use lithe_java_domain::protocol::{CoreError, ErrorCode};
use lithe_plugin_sdk::{Backend, Context, FunctionError, PluginError, SdkError, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Name reported as `serverInfo.name`.
pub const BACKEND_NAME: &str = "lithe-java-backend";

/// Builds the backend with every Java Support function registered.
pub fn backend() -> Result<Backend, SdkError> {
    let mut backend = Backend::new(BACKEND_NAME).version(env!("CARGO_PKG_VERSION"));
    for command in SUPPORTED_COMMANDS.iter().chain(SESSION_COMMANDS) {
        let command = *command;
        backend.function(command, move |params, context| {
            invoke(command, params, context)
        })?;
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
    let result = commands::execute_with_options(
        command,
        params,
        ExecuteOptions {
            operation_id: Some(operation_id),
            timeout_milliseconds: None,
        },
    );
    if cancelled.load(Ordering::SeqCst) {
        return Err(FunctionError::Cancelled);
    }
    result.map_err(|error| function_error(error, context))
}

/// Maps a domain failure to the error the host receives.
///
/// `details` may carry paths or tool output, so it only reaches stderr.
fn function_error(error: CoreError, context: &Context) -> FunctionError {
    if error.code == ErrorCode::Cancelled {
        return FunctionError::Cancelled;
    }
    if let Some(details) = &error.details {
        context.log(format!(
            "{} failed: {}: {details}",
            context.function_id(),
            error.message
        ));
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

    #[test]
    fn every_command_is_a_valid_function_id() {
        for command in SUPPORTED_COMMANDS.iter().chain(SESSION_COMMANDS) {
            assert!(lithe_plugin_sdk::is_function_id(command), "{command}");
        }
        assert!(backend().is_ok());
    }
}
