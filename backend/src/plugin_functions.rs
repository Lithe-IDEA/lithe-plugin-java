//! Cross-plugin function calls of the backend (API-35).
//!
//! A plugin function is private to the plugin that registered it. The host
//! relays calls between plugins (this plugin, Core, the other plugin), but
//! only for functions the receiver exports and the caller declares. This
//! module is the one place that knows the relay's wire: the host method
//! names, their params and their results. If the relay protocol is renamed
//! or reshaped, only this file changes.
//!
//! What it does for the Java plugin: before JDT LS starts, ask every enabled
//! provider of `languageImport.settings` (a Maven or Gradle plugin, never
//! named here) for the settings JDT LS should import with, and collect the
//! answers in [`Contributions`]. The interface itself is defined by
//! `lithe_java_domain::lsp::languages::jdt_contributions`.
//!
//! TODO(host-api: API-35 cross-plugin function): the relay was merged in
//! Lithe #1742 and its real wire differs from the shapes this module still
//! uses, so the module cannot call it yet:
//!
//! - `lithe/host/functions/providers` takes `{ functionID }` and answers
//!   `{ providers: [{ pluginID, version }] }`.
//! - `lithe/host/functions/invoke` takes
//!   `{ pluginID, functionID, context: { contextID, generation }, params?,
//!   timeoutMilliseconds?, parentOperationID? }` and answers `{ value }`.
//!   Fields the host does not define are refused with
//!   `plugin.host.unsupportedOption`.
//!
//! Both methods require plugin function protocol 4, while Rust SDK 0.2.0
//! negotiates only protocols 1 to 3. The method constants and the two
//! `*_params` / parse functions therefore change together with an upgrade to
//! an SDK that speaks protocol 4; the collector and its tests do not change.
//!
//! TODO(host-api: API-35 cross-plugin function): the change function
//! `languageImport.settingsChanged` is not registered yet. It must store the
//! new settings in the slot of the *calling plugin*. Protocol 4 names the
//! caller in the `caller` field of an invocation, and SDK 0.2.0 does not
//! expose that field to a handler. Register the function with
//! `jdt_contributions::SETTINGS_CHANGED_FUNCTION` and [`record_answer`] once
//! the SDK exposes the caller.

use lithe_java_domain::lsp::languages::jdt_contributions::{
    accept_answer, settings_request, Contributions, FunctionContract, MAX_PROVIDERS,
    SETTINGS_FUNCTION,
};
use lithe_plugin_sdk::{
    sdk_codes, Cancellation, Cancelled, Context, FunctionError, Host, PluginError, SdkError, Value,
    WorkspaceContext,
};
use serde_json::json;
use std::thread;
use std::time::Duration;

/// Host method that lists the enabled plugins exporting a function.
pub const PROVIDERS_METHOD: &str = "lithe/host/functions/providers";

/// Host method that calls an exported function of another plugin.
pub const INVOKE_METHOD: &str = "lithe/host/functions/invoke";

/// Longest wait for one provider before JDT LS starts without its settings.
/// The host applies its own, larger limit on top.
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

/// Every category this module reports. Each one is a controlled code that
/// names no provider, path or message.
pub const RELAY_CATEGORIES: &[&str] = &[
    RELAY_PROVIDERS_UNAVAILABLE,
    RELAY_PROVIDER_FAILED,
    RELAY_PROVIDER_TIMED_OUT,
    RELAY_SETTINGS_REJECTED,
    RELAY_KEYS_REJECTED,
];

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
    /// function was cancelled, or only this request ran out of time.
    fn is_cancelled(&self) -> bool {
        self.code == sdk_codes::CALL_CANCELLED
    }
}

impl From<SdkError> for RelayFailure {
    fn from(error: SdkError) -> Self {
        Self::new(error.code(), error.message())
    }
}

/// The two relay requests the collector needs.
pub trait FunctionRelay {
    /// Plugin IDs of the enabled plugins that export `function`.
    fn providers(&self, function: &FunctionContract) -> Result<Vec<String>, RelayFailure>;

    /// Calls `function` of `provider` and returns the function's result.
    fn invoke(
        &self,
        provider: &str,
        function: &FunctionContract,
        params: Value,
    ) -> Result<Value, RelayFailure>;
}

/// The relay as the host offers it to one running function call.
pub struct HostRelay<'a> {
    host: &'a Host,
    workspace: &'a WorkspaceContext,
    call_timeout: Duration,
}

impl<'a> HostRelay<'a> {
    pub fn new(host: &'a Host, workspace: &'a WorkspaceContext, call_timeout: Duration) -> Self {
        Self {
            host,
            workspace,
            call_timeout,
        }
    }

    /// Adds the workspace context every relayed request names.
    fn scoped(&self, mut params: Value) -> Value {
        params["context"] = self.workspace.to_value();
        params
    }
}

impl FunctionRelay for HostRelay<'_> {
    fn providers(&self, function: &FunctionContract) -> Result<Vec<String>, RelayFailure> {
        let params = self.scoped(providers_params(function));
        let result = self.host.request(PROVIDERS_METHOD, params)?;
        parse_providers(&result)
    }

    fn invoke(
        &self,
        provider: &str,
        function: &FunctionContract,
        params: Value,
    ) -> Result<Value, RelayFailure> {
        // The deadline cancels only this request, so one slow provider does
        // not end the whole function call.
        let attempt = Cancellation::new();
        let _deadline = Deadline::start(&attempt, self.call_timeout);
        let params = self.scoped(invoke_params(provider, function, params));
        let result = self
            .host
            .with_cancellation(&attempt)
            .request(INVOKE_METHOD, params)?;
        parse_invoke_result(result)
    }
}

/// Params of [`PROVIDERS_METHOD`], without the workspace context.
fn providers_params(function: &FunctionContract) -> Value {
    json!({ "functionID": function.id, "version": function.version })
}

/// Params of [`INVOKE_METHOD`], without the workspace context.
fn invoke_params(provider: &str, function: &FunctionContract, params: Value) -> Value {
    json!({
        "pluginID": provider,
        "functionID": function.id,
        "version": function.version,
        "params": params,
    })
}

fn parse_providers(result: &Value) -> Result<Vec<String>, RelayFailure> {
    let entries = result
        .get("providers")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_result(PROVIDERS_METHOD))?;
    entries
        .iter()
        .map(|entry| {
            entry
                .get("pluginID")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| invalid_result(PROVIDERS_METHOD))
        })
        .collect()
}

fn parse_invoke_result(result: Value) -> Result<Value, RelayFailure> {
    match result {
        Value::Object(mut fields) => fields
            .remove("result")
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

/// Cancels one request after a timeout, unless the request ends first.
///
/// Dropping the guard cancels the token, which wakes and ends the timer
/// thread; the token is not used again afterwards.
struct Deadline {
    attempt: Cancellation,
}

impl Deadline {
    fn start(attempt: &Cancellation, timeout: Duration) -> Self {
        let watched = attempt.clone();
        thread::spawn(move || {
            // `wait_timeout` answers whether the token was cancelled; only a
            // token that is still live after the timeout needs cancelling.
            if !watched.wait_timeout(timeout) {
                watched.cancel();
            }
        });
        Self {
            attempt: attempt.clone(),
        }
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.attempt.cancel();
    }
}

/// Collects the settings every enabled `languageImport.settings` provider
/// contributes for `workspace_root`.
///
/// A provider that fails, times out or answers with something unusable is
/// recorded as failed and does not stop the others, so JDT LS can always
/// start. The only error is the calling function's own cancellation.
/// `report` receives the category and count of every failure that was handled
/// this way (one of [`RELAY_CATEGORIES`]), and never a provider ID, a path or
/// a message.
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
    providers.dedup();
    providers.truncate(MAX_PROVIDERS);
    for provider in providers {
        cancellation.check()?;
        let request = settings_request(workspace_root);
        match relay.invoke(&provider, &SETTINGS_FUNCTION, request) {
            Ok(answer) => record_answer(&mut contributions, &provider, &answer, report),
            Err(failure) if failure.is_cancelled() => {
                // The calling function was cancelled, or only this request
                // ran out of time.
                cancellation.check()?;
                report(RELAY_PROVIDER_TIMED_OUT, 1);
                contributions.record_failure(&provider, "The call timed out.");
            }
            Err(failure) => {
                report(RELAY_PROVIDER_FAILED, 1);
                contributions.record_failure(&provider, failure_reason(&failure));
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

/// Collects the contributions for the workspace a function call runs for.
///
/// This is the call the language-server launch resolver makes before JDT LS
/// starts; its result feeds `JdtSettings::contributed`.
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
    let relay = HostRelay::new(context.host(), workspace, PROVIDER_TIMEOUT);
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
        providers: Result<Vec<String>, RelayFailure>,
        answers: BTreeMap<String, Result<Value, RelayFailure>>,
        calls: RefCell<Vec<(String, Value)>>,
        /// Cancels this token when a provider is called, like a host that
        /// cancels the whole function call mid-request.
        cancel_on_invoke: Option<Cancellation>,
    }

    impl FakeRelay {
        fn new(providers: &[&str]) -> Self {
            Self {
                providers: Ok(providers.iter().map(|id| id.to_string()).collect()),
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
        fn providers(&self, _function: &FunctionContract) -> Result<Vec<String>, RelayFailure> {
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
    fn the_request_params_name_the_function_and_its_contract_version() {
        assert_eq!(
            providers_params(&SETTINGS_FUNCTION),
            json!({ "functionID": "languageImport.settings", "version": 1 })
        );
        assert_eq!(
            invoke_params(
                "io.github.lithe-idea.maven-build",
                &SETTINGS_FUNCTION,
                json!({ "workspaceRoot": "/work/app" })
            ),
            json!({
                "pluginID": "io.github.lithe-idea.maven-build",
                "functionID": "languageImport.settings",
                "version": 1,
                "params": { "workspaceRoot": "/work/app" },
            })
        );
    }

    #[test]
    fn provider_lists_and_invoke_results_must_have_the_expected_shape() {
        assert_eq!(
            parse_providers(
                &json!({ "providers": [{ "pluginID": "a.b" }, { "pluginID": "c.d" }] })
            ),
            Ok(vec!["a.b".to_string(), "c.d".to_string()])
        );
        for malformed in [
            json!({}),
            json!({ "providers": "a.b" }),
            json!({ "providers": [{ "id": "a.b" }] }),
            json!({ "providers": [{ "pluginID": 7 }] }),
        ] {
            let failure = parse_providers(&malformed).expect_err("malformed list");
            assert_eq!(failure.code, "java.relay.invalidResult");
        }

        assert_eq!(
            parse_invoke_result(json!({ "result": { "settings": {} } })),
            Ok(json!({ "settings": {} }))
        );
        assert!(parse_invoke_result(json!({})).is_err());
        assert!(parse_invoke_result(json!("done")).is_err());
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
            Err(RelayFailure::new(
                "plugin.host.timeout",
                "/home/dev/.gradle is slow",
            )),
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
                "The call failed (plugin.host.timeout)."
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
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Err(RelayFailure::new(
                "plugin.host.timeout",
                "/home/dev/.gradle is slow",
            )),
        )
        // The request was cancelled but the calling function was not.
        .answering("io.github.lithe-idea.maven-build", Err(cancelled()));
        let (_, reports) = collect_reporting(&relay);
        // Providers are called in plugin ID order: gradle first, then maven.
        assert_eq!(
            reports,
            [
                (RELAY_PROVIDER_FAILED.to_string(), 1),
                (RELAY_PROVIDER_TIMED_OUT.to_string(), 1),
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
        let relay = FakeRelay::new(&[
            "io.github.lithe-idea.gradle-build",
            "io.github.lithe-idea.maven-build",
            "io.github.lithe-idea.sbt-build",
            "io.github.lithe-idea.zeta-build",
        ])
        .answering(
            "io.github.lithe-idea.gradle-build",
            Err(RelayFailure::new(
                "plugin.host.timeout",
                "/home/dev/.gradle is slow",
            )),
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
            ]
        );
        let forbidden = [
            "io.github",
            "lithe-idea",
            "gradle",
            "maven",
            "sbt",
            "zeta",
            "/home/dev",
            "/opt/other-jdk",
            "java.home",
            "unrelated",
            "plugin.host.timeout",
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
        ] {
            assert!(seen.contains(category), "{category} is not listed");
        }
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
    fn a_deadline_cancels_its_request_and_dropping_it_stops_the_timer() {
        let expired = Cancellation::new();
        let _deadline = Deadline::start(&expired, Duration::ZERO);
        // The wait ends as soon as the timer thread cancels; the bound only
        // matters when the deadline is broken.
        assert!(expired.wait_timeout(Duration::from_secs(30)));

        let finished = Cancellation::new();
        drop(Deadline::start(&finished, Duration::from_secs(3600)));
        assert!(finished.is_cancelled());
    }
}
