//! Cross-plugin function calls of the backend (API-35).
//!
//! A plugin function is private to the plugin that registered it. The host
//! relays calls between plugins (this plugin, Core, the other plugin), but
//! only for functions the receiver exports and the caller declares. This
//! module is the one place that knows what the relay answers: it sends the
//! two relay requests through the SDK's `Host::functions()` service, which
//! owns the wire shape of their params, and parses their results. If the
//! relay protocol is reshaped, only this file changes.
//!
//! What it does for the Java plugin: once JDT LS is running, ask every enabled
//! provider of `languageImport.settings` (a Maven or Gradle plugin, never
//! named here) for the settings JDT LS should import with, and collect the
//! answers in [`Contributions`]. The interface itself is defined by
//! `lithe_java_domain::lsp::languages::jdt_contributions`. The
//! [`settings_session`](crate::settings_session) module decides when to
//! collect and what to do with the result.
//!
//! The relay requires plugin function protocol 4, which Rust SDK 0.3.0
//! negotiates. A host that offers an older protocol answers the discovery
//! with `plugin.host.unavailable`; the collector records that as a failed
//! discovery and JDT LS starts without contributed settings.

use lithe_java_domain::lsp::languages::jdt_contributions::{
    accept_answer, settings_request, Contributions, FunctionContract, MAX_PROVIDERS,
    SETTINGS_FUNCTION,
};
use lithe_plugin_sdk::{
    sdk_codes, Cancellation, Cancelled, Context, FunctionError, Host, PluginError, SdkError, Value,
    WorkspaceContext,
};
use serde_json::json;
use std::time::Duration;

/// Host method that lists the enabled plugins exporting a function. The SDK
/// sends it; the name appears here only in diagnostics about its answer.
pub const PROVIDERS_METHOD: &str = "lithe/host/functions/providers";

/// Host method that calls an exported function of another plugin. The SDK
/// sends it; the name appears here only in diagnostics about its answer.
pub const INVOKE_METHOD: &str = "lithe/host/functions/invoke";

/// Longest wait for one provider; JDT LS then gets the settings without it.
/// It is sent to the host as the call's `timeoutMilliseconds`, so the host
/// ends a slow provider's call and answers `plugin.host.timeout`.
pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);

/// Category reported when the host could not list the settings providers.
pub const RELAY_PROVIDERS_UNAVAILABLE: &str = "relay.providers_unavailable";

/// Category reported when a call to one settings provider failed.
pub const RELAY_PROVIDER_FAILED: &str = "relay.provider_failed";

/// Category reported when a call to one settings provider timed out.
pub const RELAY_PROVIDER_TIMED_OUT: &str = "relay.provider_timed_out";

/// Category reported when a provider's answer was unusable and was dropped.
pub const RELAY_SETTINGS_REJECTED: &str = "relay.settings_rejected";

/// Category reported when an answer carried keys outside the accepted
/// namespace. The count is the number of rejected keys.
pub const RELAY_KEYS_REJECTED: &str = "relay.keys_rejected";

/// Category reported when a provider exports another version of
/// `languageImport.settings` than this plugin speaks, so it was not called.
pub const RELAY_PROVIDER_VERSION_MISMATCH: &str = "relay.provider_version_mismatch";

/// Every category this module reports. Each one is a controlled code that
/// names no provider, path or message.
pub const RELAY_CATEGORIES: &[&str] = &[
    RELAY_PROVIDERS_UNAVAILABLE,
    RELAY_PROVIDER_FAILED,
    RELAY_PROVIDER_TIMED_OUT,
    RELAY_SETTINGS_REJECTED,
    RELAY_KEYS_REJECTED,
    RELAY_PROVIDER_VERSION_MISMATCH,
];

/// The code of a host request that the host ended because its deadline
/// passed.
const HOST_TIMEOUT_CODE: &str = "plugin.host.timeout";

/// A relay request that was refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayFailure {
    /// Stable code, such as `plugin.host.timeout` or an `sdk.*` code.
    pub code: String,
    /// Diagnostic text. It may name paths, so it never leaves the backend.
    pub message: String,
}

impl RelayFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Whether the request was cut short by cancellation: either the calling
    /// function was cancelled, or only this request was cancelled.
    fn is_cancelled(&self) -> bool {
        self.code == sdk_codes::CALL_CANCELLED
    }

    /// Whether the host ended the request because its deadline passed.
    fn is_timed_out(&self) -> bool {
        self.code == HOST_TIMEOUT_CODE
    }
}

impl From<SdkError> for RelayFailure {
    fn from(error: SdkError) -> Self {
        Self::new(error.code(), error.message())
    }
}

/// One enabled plugin that exports a function, as the host lists it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Provider {
    /// ID of the exporting plugin.
    pub plugin_id: String,
    /// Version of the function the plugin exports. The caller can only use a
    /// provider whose version is the one its own contract names.
    pub version: u64,
}

/// The two relay requests the collector needs.
pub trait FunctionRelay {
    /// The enabled plugins that export `function`, with the version each
    /// one exports.
    fn providers(&self, function: &FunctionContract) -> Result<Vec<Provider>, RelayFailure>;

    /// Calls `function` of `provider` and returns the function's result.
    fn invoke(
        &self,
        provider: &str,
        function: &FunctionContract,
        params: Value,
    ) -> Result<Value, RelayFailure>;
}

/// The relay as the host offers it for one workspace.
///
/// It owns its host handle and workspace context, so it serves both a running
/// function call (`Context::host()`, whose cross-plugin calls belong to that
/// call) and background work (`BackendHandle::host()`).
pub struct HostRelay {
    host: Host,
    workspace: WorkspaceContext,
    call_timeout: Duration,
}

impl HostRelay {
    pub fn new(host: Host, workspace: WorkspaceContext, call_timeout: Duration) -> Self {
        Self {
            host,
            workspace,
            call_timeout,
        }
    }
}

impl FunctionRelay for HostRelay {
    fn providers(&self, function: &FunctionContract) -> Result<Vec<Provider>, RelayFailure> {
        let result = self.host.functions().providers(function.id)?;
        parse_providers(&result)
    }

    fn invoke(
        &self,
        provider: &str,
        function: &FunctionContract,
        params: Value,
    ) -> Result<Value, RelayFailure> {
        let call = invoke_call(provider, function, params, self.call_timeout);
        let result = self.host.functions().invoke(&self.workspace, call)?;
        parse_invoke_result(result)
    }
}

/// The call `Functions::invoke` relays: only the keys the host method
/// defines. The SDK adds the workspace context and, inside a function call,
/// the parent operation, and sends nothing else, so the host never refuses it
/// with `plugin.host.unsupportedOption`. The contract version is not a key:
/// the host checks it against the provider's export, and
/// [`collect_contributions`] has already skipped providers of another version.
fn invoke_call(
    provider: &str,
    function: &FunctionContract,
    params: Value,
    timeout: Duration,
) -> Value {
    json!({
        "pluginID": provider,
        "functionID": function.id,
        "params": params,
        "timeoutMilliseconds": timeout_milliseconds(timeout),
    })
}

/// The call timeout as the host takes it: a positive whole number of
/// milliseconds. A shorter wait still gets one millisecond, because the host
/// refuses zero.
fn timeout_milliseconds(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

fn parse_providers(result: &Value) -> Result<Vec<Provider>, RelayFailure> {
    let entries = result
        .get("providers")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_result(PROVIDERS_METHOD))?;
    entries
        .iter()
        .map(|entry| parse_provider(entry).ok_or_else(|| invalid_result(PROVIDERS_METHOD)))
        .collect()
}

fn parse_provider(entry: &Value) -> Option<Provider> {
    Some(Provider {
        plugin_id: entry
            .get("pluginID")?
            .as_str()
            .filter(|plugin_id| !plugin_id.is_empty())?
            .to_string(),
        version: entry.get("version")?.as_u64()?,
    })
}

/// The `value` of an invoke result. The value is the provider's own result
/// and may be `null`, so a missing `value` is a malformed answer while a
/// `null` one is returned as it is.
fn parse_invoke_result(result: Value) -> Result<Value, RelayFailure> {
    match result {
        Value::Object(mut fields) => fields
            .remove("value")
            .ok_or_else(|| invalid_result(INVOKE_METHOD)),
        _ => Err(invalid_result(INVOKE_METHOD)),
    }
}

fn invalid_result(method: &str) -> RelayFailure {
    RelayFailure::new(
        "java.relay.invalidResult",
        format!("{method} answered with an unexpected shape."),
    )
}

/// Collects the settings every enabled `languageImport.settings` provider
/// contributes for `workspace_root`.
///
/// A provider that fails, times out, exports another version or answers with
/// something unusable is recorded as failed and does not stop the others, so
/// JDT LS can always start. The only error is the calling function's own
/// cancellation. `report` receives the category and count of every failure
/// that was handled this way (one of [`RELAY_CATEGORIES`]), and never a
/// provider ID, a path or a message.
pub fn collect_contributions(
    relay: &dyn FunctionRelay,
    workspace_root: &str,
    cancellation: &Cancellation,
    report: &dyn Fn(&str, u64),
) -> Result<Contributions, Cancelled> {
    let mut contributions = Contributions::default();
    cancellation.check()?;
    let mut providers = match relay.providers(&SETTINGS_FUNCTION) {
        Ok(providers) => providers,
        Err(failure) if failure.is_cancelled() => return Err(Cancelled),
        Err(failure) => {
            report(RELAY_PROVIDERS_UNAVAILABLE, 1);
            contributions.record_discovery_failure(failure_reason(&failure));
            return Ok(contributions);
        }
    };
    providers.sort();
    providers.dedup_by(|later, kept| later.plugin_id == kept.plugin_id);
    providers.truncate(MAX_PROVIDERS);
    let expected_version = u64::from(SETTINGS_FUNCTION.version);
    for provider in providers {
        cancellation.check()?;
        if provider.version != expected_version {
            // The provider is listed, so the user should see that its
            // settings are missing, but it is never called: it would answer
            // with a shape this plugin does not know.
            report(RELAY_PROVIDER_VERSION_MISMATCH, 1);
            contributions.record_failure(
                &provider.plugin_id,
                version_mismatch_reason(provider.version, expected_version),
            );
            continue;
        }
        let request = settings_request(workspace_root);
        match relay.invoke(&provider.plugin_id, &SETTINGS_FUNCTION, request) {
            Ok(answer) => record_answer(&mut contributions, &provider.plugin_id, &answer, report),
            Err(failure) if failure.is_cancelled() || failure.is_timed_out() => {
                // Either the calling function was cancelled, which ends the
                // collection, or only this request was cut short: the host's
                // deadline for it passed, or a cancellation scoped to it.
                cancellation.check()?;
                report(RELAY_PROVIDER_TIMED_OUT, 1);
                contributions.record_failure(&provider.plugin_id, "The call timed out.");
            }
            Err(failure) => {
                report(RELAY_PROVIDER_FAILED, 1);
                contributions.record_failure(&provider.plugin_id, failure_reason(&failure));
            }
        }
    }
    Ok(contributions)
}

/// Validates one provider's answer and stores it in the provider's slot.
///
/// An unusable answer marks the provider as failed instead of keeping what it
/// sent before, so the status never claims settings that were not accepted.
/// `report` receives [`RELAY_SETTINGS_REJECTED`] for an unusable answer and
/// [`RELAY_KEYS_REJECTED`] with the number of keys dropped from a usable one;
/// it never receives the provider ID, the keys or the reason text.
pub fn record_answer(
    contributions: &mut Contributions,
    provider: &str,
    answer: &Value,
    report: &dyn Fn(&str, u64),
) {
    match accept_answer(answer) {
        Ok(accepted) => {
            if !accepted.rejected.is_empty() {
                let rejected = u64::try_from(accepted.rejected.len()).unwrap_or(u64::MAX);
                report(RELAY_KEYS_REJECTED, rejected);
            }
            contributions.record_settings(provider, accepted);
        }
        Err(reason) => {
            report(RELAY_SETTINGS_REJECTED, 1);
            contributions.record_failure(provider, reason);
        }
    }
}

/// Status-line reason for a failed request. It carries the stable code only,
/// because the message may name machine paths.
fn failure_reason(failure: &RelayFailure) -> String {
    format!("The call failed ({}).", failure.code)
}

/// Status-line reason for a provider that exports another function version.
/// Both versions are numbers, so the sentence names no path.
fn version_mismatch_reason(offered: u64, expected: u64) -> String {
    format!("The provider exports version {offered} of the function; this plugin uses version {expected}.")
}

/// Collects the contributions for the workspace a function call runs for.
///
/// Nothing calls it yet. It is kept for the backend launch resolver, which
/// will collect the contributions inside its own function call before JDT LS
/// starts and pass them in `JdtSettings::contributed`.
pub fn collect_for_call(
    context: &Context,
    workspace_root: &str,
) -> Result<Contributions, FunctionError> {
    let Some(workspace) = context.workspace_context() else {
        return Err(PluginError::new(
            "java.noWorkspaceContext",
            "The call carries no workspace context.",
        )
        .into());
    };
    let relay = HostRelay::new(context.host().clone(), workspace.clone(), PROVIDER_TIMEOUT);
    let diagnostics = context.diagnostics();
    let operation = context.function_id();
    let report = |code: &str, count: u64| {
        // An error only means the arguments were invalid; the call's own result stands.
        let _ = diagnostics.degraded(operation, code, Some(count));
    };
    Ok(collect_contributions(
        &relay,
        workspace_root,
        context.cancellation(),
        &report,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lithe_java_domain::lsp::languages::jdt::ContributedSettingsStatus;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A relay that answers from a script and records the calls it received.
    struct FakeRelay {
        providers: Result<Vec<Provider>, RelayFailure>,
        answers: BTreeMap<String, Result<Value, RelayFailure>>,
        calls: RefCell<Vec<(String, Value)>>,
        /// Cancels this token when a provider is called, like a host that
        /// cancels the whole function call mid-request.
        cancel_on_invoke: Option<Cancellation>,
    }

    impl FakeRelay {
        /// Providers that export the version this plugin speaks.
        fn new(providers: &[&str]) -> Self {
            let version = u64::from(SETTINGS_FUNCTION.version);
            Self::exporting(
                &providers
                    .iter()
                    .map(|id| (*id, version))
                    .collect::<Vec<_>>(),
            )
        }

        /// Providers with the version each one exports.
        fn exporting(providers: &[(&str, u64)]) -> Self {
            Self {
                providers: Ok(providers
                    .iter()
                    .map(|(id, version)| Provider {
                        plugin_id: id.to_string(),
                        version: *version,
                    })
                    .collect()),
                answers: BTreeMap::new(),
                calls: RefCell::new(Vec::new()),
                cancel_on_invoke: None,
            }
        }

        fn answering(mut self, provider: &str, answer: Result<Value, RelayFailure>) -> Self {
            self.answers.insert(provider.to_string(), answer);
            self
        }

        fn called_providers(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|(provider, _)| provider.clone())
                .collect()
        }
    }

    impl FunctionRelay for FakeRelay {
        fn providers(&self, _function: &FunctionContract) -> Result<Vec<Provider>, RelayFailure> {
            self.providers.clone()
        }

        fn invoke(
            &self,
            provider: &str,
            _function: &FunctionContract,
            params: Value,
        ) -> Result<Value, RelayFailure> {
            self.calls.borrow_mut().push((provider.to_string(), params));
            if let Some(cancellation) = &self.cancel_on_invoke {
                cancellation.cancel();
            }
            self.answers
                .get(provider)
                .cloned()
                .unwrap_or_else(|| Err(RelayFailure::new("plugin.host.notFound", "No provider.")))
        }
    }

    fn settings(entries: Value) -> Result<Value, RelayFailure> {
        Ok(json!({ "settings": entries }))
    }

    fn cancelled() -> RelayFailure {
        RelayFailure::new(sdk_codes::CALL_CANCELLED, "Cancelled.")
    }

    /// A provider call the host ended because its deadline passed.
    fn timed_out() -> RelayFailure {
        RelayFailure::new(HOST_TIMEOUT_CODE, "The provider took too long.")
    }

    /// A provider call that failed for a reason other than time.
    fn failed(message: &str) -> RelayFailure {
        RelayFailure::new("plugin.function.failed", message)
    }

    fn collect(relay: &FakeRelay) -> Contributions {
        collect_contributions(relay, "/work/app", &Cancellation::new(), &|_, _| {})
            .expect("the call is not cancelled")
    }

    /// Collects with a reporter that records every category and count.
    fn collect_reporting(relay: &FakeRelay) -> (Contributions, Vec<(String, u64)>) {
        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let contributions =
            collect_contributions(relay, "/work/app", &Cancellation::new(), &|code, count| {
                reports.borrow_mut().push((code.to_string(), count))
            })
            .expect("the call is not cancelled");
        (contributions, reports.into_inner())
    }

    #[test]
    fn the_interface_functions_are_valid_function_ids() {
        use lithe_java_domain::lsp::languages::jdt_contributions::SETTINGS_CHANGED_FUNCTION;
        for function in [SETTINGS_FUNCTION, SETTINGS_CHANGED_FUNCTION] {
            assert!(
                lithe_plugin_sdk::is_function_id(function.id),
                "{}",
                function.id
            );
        }
    }

    #[test]
    fn the_invoke_call_carries_only_the_keys_the_relay_defines() {
        let call = invoke_call(
            "io.github.lithe-idea.maven-build",
            &SETTINGS_FUNCTION,
            json!({ "workspaceRoot": "/work/app" }),
            PROVIDER_TIMEOUT,
        );
        assert_eq!(
            call,
            json!({
                "pluginID": "io.github.lithe-idea.maven-build",
                "functionID": "languageImport.settings",
                "params": { "workspaceRoot": "/work/app" },
                "timeoutMilliseconds": 10_000,
            })
        );
        // The version and the context are not keys of the call: the host
        // refuses a key it does not define, and the SDK adds the context.
        let allowed = [
            "pluginID",
            "functionID",
            "params",
            "timeoutMilliseconds",
            "parentOperationID",
        ];
        for key in call.as_object().expect("the call is an object").keys() {
            assert!(allowed.contains(&key.as_str()), "{key}");
        }
    }

    #[test]
    fn the_call_timeout_is_a_positive_whole_number_of_milliseconds() {
        assert_eq!(timeout_milliseconds(PROVIDER_TIMEOUT), 10_000);
        assert_eq!(timeout_milliseconds(Duration::from_millis(1500)), 1500);
        // Sub-millisecond and zero waits still ask for one millisecond.
        assert_eq!(timeout_milliseconds(Duration::from_nanos(1)), 1);
        assert_eq!(timeout_milliseconds(Duration::ZERO), 1);
        assert_eq!(timeout_milliseconds(Duration::MAX), u64::MAX);
    }

    #[test]
    fn provider_lists_and_invoke_results_must_have_the_expected_shape() {
        assert_eq!(
            parse_providers(&json!({ "providers": [
                { "pluginID": "a.b", "version": 1 },
                { "pluginID": "c.d", "version": 2 },
            ] })),
            Ok(vec![
                Provider {
                    plugin_id: "a.b".to_string(),
                    version: 1
                },
                Provider {
                    plugin_id: "c.d".to_string(),
                    version: 2
                },
            ])
        );
        assert_eq!(parse_providers(&json!({ "providers": [] })), Ok(vec![]));
        for malformed in [
            json!({}),
            json!({ "providers": "a.b" }),
            json!({ "providers": [{ "id": "a.b", "version": 1 }] }),
            json!({ "providers": [{ "pluginID": 7, "version": 1 }] }),
            json!({ "providers": [{ "pluginID": "", "version": 1 }] }),
            json!({ "providers": [{ "pluginID": "a.b" }] }),
            json!({ "providers": [{ "pluginID": "a.b", "version": "1" }] }),
            json!({ "providers": [{ "pluginID": "a.b", "version": -1 }] }),
        ] {
            let failure = parse_providers(&malformed).expect_err("malformed list");
            assert_eq!(failure.code, "java.relay.invalidResult", "{malformed}");
        }

        assert_eq!(
            parse_invoke_result(json!({ "value": { "settings": {} } })),
            Ok(json!({ "settings": {} }))
        );
        // The key of the result is `value`; the draft protocol called it `result`.
        assert!(parse_invoke_result(json!({ "result": { "settings": {} } })).is_err());
        assert!(parse_invoke_result(json!({})).is_err());
        assert!(parse_invoke_result(json!("done")).is_err());
    }

    #[test]
    fn a_null_value_is_a_result_and_a_missing_value_is_not() {
        assert_eq!(
            parse_invoke_result(json!({ "value": null })),
            Ok(Value::Null)
        );
        let failure = parse_invoke_result(json!({ "other": null })).expect_err("no value");
        assert_eq!(failure.code, "java.relay.invalidResult");
    }

    #[test]
    fn a_provider_that_answers_null_is_recorded_as_failed() {
        let relay = FakeRelay::new(&["io.github.lithe-idea.maven-build"])
            .answering("io.github.lithe-idea.maven-build", Ok(Value::Null));
        let (contributions, reports) = collect_reporting(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(contributions.failures().len(), 1);
        assert!(contributions.fragments().is_empty());
        assert_eq!(reports, [(RELAY_SETTINGS_REJECTED.to_string(), 1)]);
    }

    #[test]
    fn providers_are_called_once_each_in_plugin_id_order() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.maven-build",
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering("io.github.lithe-idea.maven-build", settings(json!({})))
        .answering("io.github.lithe-idea.gradle-build", settings(json!({})));
        collect(&relay);
        assert_eq!(
            relay.called_providers(),
            [
                "io.github.lithe-idea.gradle-build",
                "io.github.lithe-idea.maven-build"
            ]
        );
        for (_, params) in relay.calls.borrow().iter() {
            assert_eq!(params, &json!({ "workspaceRoot": "/work/app" }));
        }
    }

    #[test]
    fn a_provider_of_another_version_is_not_called_and_is_reported() {
        let relay = FakeRelay::exporting(&[
            ("io.github.lithe-idea.gradle-build", 2),
            ("io.github.lithe-idea.maven-build", 1),
            ("io.github.lithe-idea.sbt-build", 0),
        ])
        .answering("io.github.lithe-idea.gradle-build", settings(json!({})))
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        )
        .answering("io.github.lithe-idea.sbt-build", settings(json!({})));
        let (contributions, reports) = collect_reporting(&relay);

        // Only the provider of the contract version is called.
        assert_eq!(
            relay.called_providers(),
            ["io.github.lithe-idea.maven-build"]
        );
        assert_eq!(
            contributions.fragments(),
            [json!({ "java.import.maven.enabled": true })]
        );
        // The others are failed providers, so the status line names them.
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(
            contributions.failures(),
            [
                (
                    "io.github.lithe-idea.gradle-build",
                    "The provider exports version 2 of the function; this plugin uses version 1."
                ),
                (
                    "io.github.lithe-idea.sbt-build",
                    "The provider exports version 0 of the function; this plugin uses version 1."
                ),
            ]
        );
        assert_eq!(
            reports,
            [
                (RELAY_PROVIDER_VERSION_MISMATCH.to_string(), 1),
                (RELAY_PROVIDER_VERSION_MISMATCH.to_string(), 1),
            ]
        );
    }

    #[test]
    fn answers_are_merged_in_plugin_id_order() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.maven-build",
            "io.github.lithe-idea.gradle-build",
        ])
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.configuration.maven.userSettings": "/m/settings.xml" })),
        )
        .answering(
            "io.github.lithe-idea.gradle-build",
            settings(json!({ "java.import.gradle.offline.enabled": true })),
        );
        let contributions = collect(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);
        assert_eq!(
            contributions.fragments(),
            [
                json!({ "java.import.gradle.offline.enabled": true }),
                json!({ "java.configuration.maven.userSettings": "/m/settings.xml" }),
            ]
        );
    }

    #[test]
    fn no_provider_means_a_plain_java_workspace() {
        let relay = FakeRelay::new(&[]);
        let contributions = collect(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);
        assert!(contributions.fragments().is_empty());
        assert!(!contributions.has_providers());
        assert!(relay.called_providers().is_empty());
    }

    #[test]
    fn a_failing_provider_does_not_stop_the_others() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Err(failed("/home/dev/.gradle is broken")),
        )
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        );
        let contributions = collect(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(
            contributions.fragments(),
            [json!({ "java.import.maven.enabled": true })]
        );
        // The reason carries the code and never the message with its path.
        assert_eq!(
            contributions.failures(),
            [(
                "io.github.lithe-idea.gradle-build",
                "The call failed (plugin.function.failed)."
            )]
        );
    }

    #[test]
    fn an_unusable_answer_marks_only_that_provider_failed() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Ok(json!({ "settings": "offline" })),
        )
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        );
        let contributions = collect(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(contributions.failures().len(), 1);
        assert_eq!(
            contributions.failures()[0].0,
            "io.github.lithe-idea.gradle-build"
        );
        assert_eq!(contributions.fragments().len(), 1);
    }

    #[test]
    fn rejected_keys_are_reported_by_count_and_never_stored() {
        let relay = FakeRelay::new(&["io.github.lithe-idea.maven-build"]).answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({
                "java.import.maven.enabled": true,
                "java.home": "/opt/other-jdk",
                "unrelated": 1
            })),
        );
        let (contributions, reports) = collect_reporting(&relay);
        assert_eq!(
            contributions.fragments(),
            [json!({ "java.import.maven.enabled": true })]
        );
        // One report for the two rejected keys; neither key nor value is named.
        assert_eq!(reports, [(RELAY_KEYS_REJECTED.to_string(), 2)]);
    }

    #[test]
    fn a_failed_discovery_is_reported_once_without_its_message() {
        let mut relay = FakeRelay::new(&[]);
        relay.providers = Err(RelayFailure::new(
            "plugin.host.unavailable",
            "No relay at /home/dev/relay.sock.",
        ));
        let (_, reports) = collect_reporting(&relay);
        assert_eq!(reports, [(RELAY_PROVIDERS_UNAVAILABLE.to_string(), 1)]);
    }

    #[test]
    fn a_failed_and_a_timed_out_provider_are_reported_by_category() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
            "io.github.lithe-idea.sbt-build",
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Err(failed("/home/dev/.gradle is broken")),
        )
        // The request was cancelled but the calling function was not.
        .answering("io.github.lithe-idea.maven-build", Err(cancelled()))
        // The host ended the request when its deadline passed.
        .answering("io.github.lithe-idea.sbt-build", Err(timed_out()));
        let (contributions, reports) = collect_reporting(&relay);
        // Providers are called in plugin ID order: gradle, maven, then sbt.
        assert_eq!(
            reports,
            [
                (RELAY_PROVIDER_FAILED.to_string(), 1),
                (RELAY_PROVIDER_TIMED_OUT.to_string(), 1),
                (RELAY_PROVIDER_TIMED_OUT.to_string(), 1),
            ]
        );
        assert_eq!(
            contributions.failures(),
            [
                (
                    "io.github.lithe-idea.gradle-build",
                    "The call failed (plugin.function.failed)."
                ),
                ("io.github.lithe-idea.maven-build", "The call timed out."),
                ("io.github.lithe-idea.sbt-build", "The call timed out."),
            ]
        );
    }

    #[test]
    fn an_unusable_answer_is_reported_as_rejected_settings() {
        let relay = FakeRelay::new(&["io.github.lithe-idea.gradle-build"]).answering(
            "io.github.lithe-idea.gradle-build",
            Ok(json!({ "settings": "offline" })),
        );
        let (_, reports) = collect_reporting(&relay);
        assert_eq!(reports, [(RELAY_SETTINGS_REJECTED.to_string(), 1)]);
    }

    #[test]
    fn a_clean_collection_reports_nothing() {
        let relay = FakeRelay::new(&["io.github.lithe-idea.maven-build"]).answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        );
        let (contributions, reports) = collect_reporting(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);
        assert!(reports.is_empty(), "{reports:?}");
    }

    #[test]
    fn reports_never_name_a_provider_a_path_or_a_key() {
        let relay = FakeRelay::exporting(&[
            ("io.github.lithe-idea.gradle-build", 1),
            ("io.github.lithe-idea.maven-build", 1),
            ("io.github.lithe-idea.sbt-build", 1),
            ("io.github.lithe-idea.zeta-build", 1),
            ("io.github.lithe-idea.zulu-build", 3),
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Err(failed("/home/dev/.gradle is broken")),
        )
        .answering("io.github.lithe-idea.maven-build", Err(cancelled()))
        .answering(
            "io.github.lithe-idea.sbt-build",
            Ok(json!({ "settings": "offline" })),
        )
        .answering(
            "io.github.lithe-idea.zeta-build",
            settings(json!({
                "java.import.maven.enabled": true,
                "java.home": "/opt/other-jdk",
                "unrelated": 1
            })),
        );
        let (_, reports) = collect_reporting(&relay);
        assert_eq!(
            reports,
            [
                (RELAY_PROVIDER_FAILED.to_string(), 1),
                (RELAY_PROVIDER_TIMED_OUT.to_string(), 1),
                (RELAY_SETTINGS_REJECTED.to_string(), 1),
                (RELAY_KEYS_REJECTED.to_string(), 2),
                (RELAY_PROVIDER_VERSION_MISMATCH.to_string(), 1),
            ]
        );
        let forbidden = [
            "io.github",
            "lithe-idea",
            "gradle",
            "maven",
            "sbt",
            "zeta",
            "zulu",
            "/home/dev",
            "/opt/other-jdk",
            "java.home",
            "unrelated",
            "plugin.function.failed",
        ];
        for (code, _) in &reports {
            assert!(RELAY_CATEGORIES.contains(&code.as_str()), "{code}");
            for fragment in forbidden {
                assert!(!code.contains(fragment), "{code} names {fragment}");
            }
        }
    }

    /// A byte the host accepts in a diagnostic code: an ASCII letter or digit,
    /// or one of `._:/-`.
    fn is_controlled_code_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    }

    #[test]
    fn the_categories_are_distinct_controlled_codes() {
        let mut seen = std::collections::BTreeSet::new();
        for category in RELAY_CATEGORIES {
            assert!(!category.is_empty());
            assert!(category.len() <= 64, "{category}");
            assert!(category.bytes().all(is_controlled_code_byte), "{category}");
            assert!(seen.insert(*category), "{category} is listed twice");
        }
        for category in [
            RELAY_PROVIDERS_UNAVAILABLE,
            RELAY_PROVIDER_FAILED,
            RELAY_PROVIDER_TIMED_OUT,
            RELAY_SETTINGS_REJECTED,
            RELAY_KEYS_REJECTED,
            RELAY_PROVIDER_VERSION_MISMATCH,
        ] {
            assert!(seen.contains(category), "{category} is not listed");
        }
        assert_eq!(seen.len(), 6);
    }

    #[test]
    fn a_failed_discovery_still_lets_the_server_start() {
        let mut relay = FakeRelay::new(&[]);
        relay.providers = Err(RelayFailure::new("plugin.host.unavailable", "No relay."));
        let contributions = collect(&relay);
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(
            contributions.discovery_failure(),
            Some("The call failed (plugin.host.unavailable).")
        );
        assert!(!contributions.has_providers());
        assert!(relay.called_providers().is_empty());
    }

    #[test]
    fn at_most_max_providers_are_asked() {
        let ids: Vec<String> = (0..MAX_PROVIDERS + 4)
            .map(|index| format!("io.github.lithe-idea.p{index:02}"))
            .collect();
        let relay = FakeRelay::new(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        collect(&relay);
        assert_eq!(relay.called_providers().len(), MAX_PROVIDERS);
    }

    #[test]
    fn a_request_that_ran_out_of_time_fails_only_its_provider() {
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering("io.github.lithe-idea.gradle-build", Err(cancelled()))
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        );
        let contributions = collect(&relay);
        assert_eq!(
            contributions.failures(),
            [("io.github.lithe-idea.gradle-build", "The call timed out.")]
        );
        assert_eq!(contributions.fragments().len(), 1);
    }

    #[test]
    fn cancelling_the_function_call_ends_the_collection() {
        let cancellation = Cancellation::new();
        cancellation.cancel();
        let relay = FakeRelay::new(&["io.github.lithe-idea.maven-build"]);
        let outcome = collect_contributions(&relay, "/work/app", &cancellation, &|_, _| {});
        assert!(outcome.is_err());
        assert!(relay.called_providers().is_empty());

        // Cancelled while a provider is being called: stop before the next one.
        let cancellation = Cancellation::new();
        let mut relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering("io.github.lithe-idea.gradle-build", Err(cancelled()));
        relay.cancel_on_invoke = Some(cancellation.clone());
        let outcome = collect_contributions(&relay, "/work/app", &cancellation, &|_, _| {});
        assert!(outcome.is_err());
        assert_eq!(
            relay.called_providers(),
            ["io.github.lithe-idea.gradle-build"]
        );
    }

    #[test]
    fn a_deadline_that_passed_in_the_host_does_not_end_the_collection() {
        // `plugin.host.timeout` is the host's answer to one slow provider; the
        // calling function is still live, so the next provider is asked.
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
        ])
        .answering("io.github.lithe-idea.gradle-build", Err(timed_out()))
        .answering(
            "io.github.lithe-idea.maven-build",
            settings(json!({ "java.import.maven.enabled": true })),
        );
        let outcome = collect_contributions(&relay, "/work/app", &Cancellation::new(), &|_, _| {});
        let contributions = outcome.expect("only the function's own cancellation ends it");
        assert_eq!(relay.called_providers().len(), 2);
        assert_eq!(contributions.fragments().len(), 1);
    }

    #[test]
    fn cancellation_and_a_passed_deadline_are_told_apart_by_code() {
        assert!(cancelled().is_cancelled());
        assert!(!cancelled().is_timed_out());
        assert!(timed_out().is_timed_out());
        assert!(!timed_out().is_cancelled());
        assert!(!failed("broken").is_cancelled());
        assert!(!failed("broken").is_timed_out());
    }
}
