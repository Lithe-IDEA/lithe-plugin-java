//! JSON command surface of the Java domain.
//!
//! Rust Core used to route these commands through its central dispatcher.
//! [`execute`] keeps the same command names, payload shapes, response shapes
//! and `InvalidRequest` messages, so the host and the plugin backend can move
//! a caller from Core to the plugin without changing its JSON.
//!
//! Commands that need a live JDT LS session (`java.navigationMarkers`,
//! `java.resolveNavigation` and `java.prepareLaunch`) answer `NotSupported`
//! until the host exposes the language-server request channel to plugin
//! backends.
//!
//! Core's `lsp.retryMavenProfiles` and `lsp.updateMavenConfiguration` are not
//! here: Maven profiles and settings belong to the Maven plugin, which hands
//! them to this plugin through a cross-plugin function call (API-35; see
//! `lsp::languages::jdt::CONTRIBUTED_SETTINGS_KEYS`).

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::protocol::cancellation::{self, Scope};
use crate::protocol::{CoreError, ErrorCode};

/// Command names that [`execute`] answers with domain logic.
pub const SUPPORTED_COMMANDS: &[&str] = &[
    "java.workspacePolicy",
    "java.runMarkers",
    "java.jdtCacheRetention",
    "java.jdtWorkspaceFingerprint",
    "lsp.jdtWorkspaceKey",
    "java.codeVision",
    "java.className",
    "java.sourceDefinition",
    "java.serverPort",
    "java.structure",
    "spring.index",
    "mybatis.index",
    "runConfig.selectJava",
    "debug.javaTestLaunch",
    "execution.planLaunchCommand",
];

/// Command names that Core served from a live JDT LS session.
///
/// They stay recognised so that callers receive a clear `NotSupported` error
/// instead of `UnknownCommand` while the host API is missing.
pub const SESSION_COMMANDS: &[&str] = &[
    "java.navigationMarkers",
    "java.resolveNavigation",
    "java.prepareLaunch",
];

/// Options that frame one command execution.
#[derive(Debug, Clone, Default)]
pub struct ExecuteOptions {
    /// Identifier that [`cancel`] can target while the command runs.
    pub operation_id: Option<String>,
    /// Deadline after which cooperative cancellation checks fail.
    pub timeout_milliseconds: Option<u64>,
}

/// Runs one command with default options.
pub fn execute(command: &str, payload: Value) -> Result<Value, CoreError> {
    execute_with_options(command, payload, ExecuteOptions::default())
}

/// Runs one command inside a cancellation scope.
pub fn execute_with_options(
    command: &str,
    payload: Value,
    options: ExecuteOptions,
) -> Result<Value, CoreError> {
    let _scope = Scope::begin(options.operation_id, options.timeout_milliseconds);
    cancellation::check()?;
    dispatch(command, payload)
}

/// Requests cooperative cancellation of a running command.
///
/// Returns `true` when an active scope with this operation identifier exists.
pub fn cancel(operation_id: &str) -> bool {
    cancellation::cancel(operation_id)
}

fn dispatch(command: &str, payload: Value) -> Result<Value, CoreError> {
    match command {
        "java.workspacePolicy" => run(
            payload,
            "Invalid Java workspace-policy request",
            crate::lsp::java_workspace_policy,
        ),
        "java.runMarkers" => run(
            payload,
            "Invalid Java Run-marker request",
            crate::lsp::java_run_markers,
        ),
        "java.jdtCacheRetention" => run(
            payload,
            "Invalid JDT LS cache-retention request",
            crate::lsp::jdt_cache_retention,
        ),
        "java.jdtWorkspaceFingerprint" => run(
            payload,
            "Invalid JDT LS workspace-fingerprint request",
            crate::lsp::jdt_workspace_fingerprint,
        ),
        "lsp.jdtWorkspaceKey" => run(payload, "Invalid JDT LS workspace-key request", |request| {
            Ok(crate::lsp::resolve_workspace_key(request))
        }),
        "java.codeVision" => run(
            payload,
            "Invalid Java code vision request",
            crate::languages::code_vision,
        ),
        "java.className" => run(
            payload,
            "Invalid Java class name request",
            crate::languages::class_name,
        ),
        "java.sourceDefinition" => run(
            payload,
            "Invalid Java source definition request",
            crate::languages::source_definition,
        ),
        "java.serverPort" => run(
            payload,
            "Invalid Java server port request",
            crate::languages::server_port,
        ),
        "java.structure" => run(
            payload,
            "Invalid Java structure request",
            crate::languages::structure,
        ),
        "spring.index" => run(
            payload,
            "Invalid Spring index request",
            crate::languages::spring_index,
        ),
        "mybatis.index" => run(
            payload,
            "Invalid MyBatis index request",
            crate::languages::mybatis_index,
        ),
        "runConfig.selectJava" => run(
            payload,
            "Invalid Java selection request",
            crate::execution::select_java,
        ),
        "debug.javaTestLaunch" => run(
            payload,
            "Invalid Java test debug launch request",
            crate::debug::java_test_launch,
        ),
        "execution.planLaunchCommand" => {
            run(payload, "Invalid Java launch-command request", |request| {
                Ok(crate::execution::plan_launch_command_request(request))
            })
        }
        // TODO(host-api: API-25 language server): these commands drive requests
        // through the running JDT LS session. They need a host channel that
        // lets a plugin backend send `workspace/executeCommand` and custom
        // requests to the language server it declared, and receive its
        // notifications.
        command if SESSION_COMMANDS.contains(&command) => Err(CoreError::new(
            ErrorCode::NotSupported,
            format!("{command} needs the host language-server request channel"),
        )),
        command => Err(CoreError::new(
            ErrorCode::InvalidRequest,
            format!("Unknown Java Support command: {command}"),
        )),
    }
}

fn run<Request, Response>(
    payload: Value,
    invalid_message: &str,
    handler: impl FnOnce(Request) -> Result<Response, CoreError>,
) -> Result<Value, CoreError>
where
    Request: DeserializeOwned,
    Response: Serialize,
{
    let request = serde_json::from_value::<Request>(payload).map_err(|error| {
        CoreError::new(ErrorCode::InvalidRequest, invalid_message).with_details(error.to_string())
    })?;
    let response = handler(request)?;
    serde_json::to_value(response).map_err(|error| {
        CoreError::new(ErrorCode::Unknown, "Java Support response did not encode")
            .with_details(error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_commands_are_invalid_requests() {
        let error = execute("java.unknown", json!({})).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(error.message, "Unknown Java Support command: java.unknown");
    }

    #[test]
    fn session_commands_wait_for_the_host_channel() {
        for command in SESSION_COMMANDS {
            let error = execute(command, json!({})).unwrap_err();
            assert_eq!(error.code, ErrorCode::NotSupported, "{command}");
        }
    }

    #[test]
    fn malformed_payloads_keep_core_messages() {
        let error = execute("java.className", json!({"unexpected": true})).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(error.message, "Invalid Java class name request");
        assert!(error.details.is_some());
    }

    #[test]
    fn cancelling_an_unknown_operation_reports_false() {
        assert!(!cancel("no-such-operation"));
    }
}
