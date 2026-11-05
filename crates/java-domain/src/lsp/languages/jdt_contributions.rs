//! Settings that other plugins contribute to JDT LS.
//!
//! JDT LS imports Maven and Gradle projects on its own. The values it imports
//! with (settings file, local repository, offline mode, Gradle user home and
//! so on) belong to the plugins that own those build systems, so this plugin
//! defines a small function interface and never computes them:
//!
//! - [`SETTINGS_FUNCTION`] (`languageImport.settings`) is implemented by a
//!   build-system plugin. Before JDT LS starts, the backend discovers every
//!   enabled provider through the host, calls each one with
//!   [`settings_request`] and records the answers in [`Contributions`].
//! - [`SETTINGS_CHANGED_FUNCTION`] (`languageImport.settingsChanged`) is
//!   exported by this plugin. When the user changes the build system's
//!   settings, the build-system plugin calls it and passes the complete new
//!   settings in the call. This plugin never calls back into a plugin that is
//!   in the middle of calling it, because the host rejects such a loop.
//!
//! The host relays both directions (this plugin, Core, the other plugin) and
//! passes no trust along. A provider can only set keys inside
//! [`CONTRIBUTED_SETTINGS_KEYS`]; everything else is dropped here before it
//! can reach JDT LS.
//!
//! This module is the pure part of that design: contract constants, answer
//! validation and bookkeeping. The calls themselves are in the backend's
//! `plugin_functions` module, and the push of the merged result to JDT LS is
//! in its `settings_session` module.
//!
//! Both answers and change calls carry this object:
//!
//! ```json
//! { "settings": { "java.import.maven.enabled": true } }
//! ```
//!
//! `settings` may nest objects or use dotted keys. [`accept_answer`] reads
//! only `settings` and ignores unknown sibling fields, so a later version can
//! add fields to an answer without breaking this one. A change call also
//! carries an optional `projects` list, and the backend is stricter with it:
//! it refuses params with a field this version does not define, because a
//! change applied only in part would leave JDT LS out of step with the build
//! system.

use super::jdt::{accept_contributed_settings, ContributedSettingsStatus};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// One function of the interface this plugin defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionContract {
    /// Function ID, dot-separated lowerCamelCase segments.
    pub id: &'static str,
    /// Version of the params and result this function exchanges. An
    /// incompatible change uses a new version; callers and exporters must
    /// agree on it.
    pub version: u32,
}

/// Implemented by build-system plugins: answers with the JDT LS settings
/// their projects need. Params are built by [`settings_request`].
pub const SETTINGS_FUNCTION: FunctionContract = FunctionContract {
    id: "languageImport.settings",
    version: 1,
};

/// Exported by this plugin: a build-system plugin reports that its settings
/// changed and passes the complete new settings.
pub const SETTINGS_CHANGED_FUNCTION: FunctionContract = FunctionContract {
    id: "languageImport.settingsChanged",
    version: 1,
};

/// Most providers consulted for one workspace. Further ones are ignored, so a
/// crowded plugin list cannot delay the start of JDT LS.
pub const MAX_PROVIDERS: usize = 16;

/// Params of [`SETTINGS_FUNCTION`].
pub fn settings_request(workspace_root: &str) -> Value {
    json!({ "workspaceRoot": workspace_root })
}

/// The settings of one answer after validation.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedSettings {
    /// Flat object of dotted keys, all inside the accepted namespace.
    pub fragment: Value,
    /// Leaf keys outside the accepted namespace. They never reach JDT LS.
    pub rejected: Vec<String>,
}

/// Validates an answer of [`SETTINGS_FUNCTION`] or the params of
/// [`SETTINGS_CHANGED_FUNCTION`].
///
/// The error is a diagnostic sentence for the log and for the status line;
/// it contains no values from the answer.
pub fn accept_answer(answer: &Value) -> Result<AcceptedSettings, String> {
    let settings = match answer.get("settings") {
        Some(settings) if settings.is_object() => settings,
        Some(_) => return Err("`settings` is not an object.".to_string()),
        None => return Err("The answer has no `settings` object.".to_string()),
    };
    let contributed = accept_contributed_settings(settings);
    Ok(AcceptedSettings {
        fragment: Value::Object(contributed.accepted.into_iter().collect()),
        rejected: contributed.rejected,
    })
}

/// What one provider last told this plugin.
#[derive(Debug, Clone, PartialEq)]
enum Slot {
    Settings(Value),
    Failed(String),
}

/// The latest contribution of every provider of one workspace.
///
/// Each provider owns one slot, so a change call replaces exactly what that
/// provider sent before and a removed setting disappears. Providers are
/// ordered by plugin ID, which keeps the merged result independent of the
/// order in which they answered.
#[derive(Debug, Clone, Default)]
pub struct Contributions {
    slots: BTreeMap<String, Slot>,
    discovery_failure: Option<String>,
}

impl Contributions {
    /// Stores the settings a provider answered with, replacing its slot.
    pub fn record_settings(&mut self, provider: &str, accepted: AcceptedSettings) {
        self.slots
            .insert(provider.to_string(), Slot::Settings(accepted.fragment));
    }

    /// Remembers that a provider failed or timed out, replacing its slot.
    /// `reason` is a short diagnostic that names no machine path.
    pub fn record_failure(&mut self, provider: &str, reason: impl Into<String>) {
        self.slots
            .insert(provider.to_string(), Slot::Failed(reason.into()));
    }

    /// Remembers that the host could not list the providers at all, so it is
    /// unknown which plugins should have contributed.
    pub fn record_discovery_failure(&mut self, reason: impl Into<String>) {
        self.discovery_failure = Some(reason.into());
    }

    /// Drops what a provider contributed, for example when its plugin is
    /// disabled.
    pub fn forget(&mut self, provider: &str) {
        self.slots.remove(provider);
    }

    /// Fragments to put into `JdtSettings::contributed`, in plugin ID order.
    /// When two providers set the same key, the one with the later plugin ID
    /// wins.
    pub fn fragments(&self) -> Vec<Value> {
        self.slots
            .values()
            .filter_map(|slot| match slot {
                Slot::Settings(fragment) => Some(fragment.clone()),
                Slot::Failed(_) => None,
            })
            .collect()
    }

    /// Providers that failed, with the reason, in plugin ID order.
    pub fn failures(&self) -> Vec<(&str, &str)> {
        self.slots
            .iter()
            .filter_map(|(provider, slot)| match slot {
                Slot::Failed(reason) => Some((provider.as_str(), reason.as_str())),
                Slot::Settings(_) => None,
            })
            .collect()
    }

    /// Why the providers could not be listed, if they could not.
    pub fn discovery_failure(&self) -> Option<&str> {
        self.discovery_failure.as_deref()
    }

    /// Whether any provider holds a slot, answered or failed. A workspace
    /// without providers is a plain Java workspace, and a failed discovery
    /// alone does not make a provider appear.
    pub fn has_providers(&self) -> bool {
        !self.slots.is_empty()
    }

    /// [`Failed`](ContributedSettingsStatus::Failed) while the providers
    /// could not be listed or any provider's last answer failed, otherwise
    /// [`Settled`](ContributedSettingsStatus::Settled). The collector reports
    /// [`Pending`](ContributedSettingsStatus::Pending) itself while calls are
    /// still outstanding.
    pub fn status(&self) -> ContributedSettingsStatus {
        if self.failures().is_empty() && self.discovery_failure.is_none() {
            ContributedSettingsStatus::Settled
        } else {
            ContributedSettingsStatus::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(settings: Value) -> AcceptedSettings {
        accept_answer(&json!({ "settings": settings })).expect("a valid answer")
    }

    #[test]
    fn the_request_names_the_workspace_root() {
        assert_eq!(
            settings_request("/work/app"),
            json!({ "workspaceRoot": "/work/app" })
        );
    }

    #[test]
    fn an_answer_keeps_only_the_declared_jdt_keys() {
        let answer = accepted(json!({
            "java": { "import": { "maven": { "enabled": true } } },
            "java.import.gradle.offline.enabled": true,
            "java.home": "/opt/jdk",
            "unrelated": 1
        }));
        assert_eq!(
            answer.fragment,
            json!({
                "java.import.gradle.offline.enabled": true,
                "java.import.maven.enabled": true
            })
        );
        let mut rejected = answer.rejected;
        rejected.sort();
        assert_eq!(rejected, ["java.home", "unrelated"]);
    }

    #[test]
    fn a_stored_fragment_survives_the_second_filter_unchanged() {
        let answer = accepted(json!({
            "java.configuration.maven.userSettings": "/home/dev/.m2/settings.xml",
            "java.project.sourcePaths": ["src/main/java"]
        }));
        let again = accept_contributed_settings(&answer.fragment);
        assert!(again.rejected.is_empty());
        assert_eq!(
            Value::Object(again.accepted.into_iter().collect()),
            answer.fragment
        );
    }

    #[test]
    fn an_answer_without_a_settings_object_is_refused() {
        for answer in [
            json!({}),
            json!(null),
            json!({ "settings": null }),
            json!({ "settings": ["java.import.maven.enabled"] }),
        ] {
            assert!(accept_answer(&answer).is_err(), "{answer}");
        }
    }

    #[test]
    fn unknown_sibling_fields_are_ignored() {
        let answer = accept_answer(&json!({
            "settings": { "java.import.maven.enabled": false },
            "futureField": { "any": "thing" }
        }))
        .expect("a valid answer");
        assert_eq!(
            answer.fragment,
            json!({ "java.import.maven.enabled": false })
        );
    }

    #[test]
    fn fragments_follow_plugin_id_order_not_arrival_order() {
        let mut contributions = Contributions::default();
        contributions.record_settings(
            "io.github.lithe-idea.maven-build",
            accepted(json!({ "java.import.maven.enabled": true })),
        );
        contributions.record_settings(
            "io.github.lithe-idea.gradle-build",
            accepted(json!({ "java.import.gradle.offline.enabled": true })),
        );
        assert_eq!(
            contributions.fragments(),
            [
                json!({ "java.import.gradle.offline.enabled": true }),
                json!({ "java.import.maven.enabled": true }),
            ]
        );
    }

    #[test]
    fn a_change_replaces_the_providers_slot_and_removes_old_settings() {
        let mut contributions = Contributions::default();
        let maven = "io.github.lithe-idea.maven-build";
        contributions.record_settings(
            maven,
            accepted(json!({
                "java.configuration.maven.userSettings": "/a/settings.xml",
                "java.import.maven.offline.enabled": true
            })),
        );
        contributions.record_settings(
            maven,
            accepted(json!({ "java.configuration.maven.userSettings": "/b/settings.xml" })),
        );
        assert_eq!(
            contributions.fragments(),
            [json!({ "java.configuration.maven.userSettings": "/b/settings.xml" })]
        );
    }

    #[test]
    fn a_failed_provider_marks_the_status_until_it_answers_again() {
        let mut contributions = Contributions::default();
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);

        contributions.record_failure("io.github.lithe-idea.maven-build", "The call timed out.");
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(
            contributions.failures(),
            [("io.github.lithe-idea.maven-build", "The call timed out.")]
        );
        assert!(contributions.fragments().is_empty());

        contributions.record_settings(
            "io.github.lithe-idea.maven-build",
            accepted(json!({ "java.import.maven.enabled": true })),
        );
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);
        assert!(contributions.failures().is_empty());
    }

    #[test]
    fn a_discovery_failure_marks_the_status_without_naming_a_provider() {
        let mut contributions = Contributions::default();
        contributions.record_discovery_failure("The host could not list providers.");
        assert_eq!(contributions.status(), ContributedSettingsStatus::Failed);
        assert_eq!(
            contributions.discovery_failure(),
            Some("The host could not list providers.")
        );
        assert!(contributions.failures().is_empty());
        assert!(contributions.fragments().is_empty());
    }

    #[test]
    fn a_provider_exists_once_it_has_a_slot_of_either_kind() {
        let mut contributions = Contributions::default();
        assert!(!contributions.has_providers());

        // A failed discovery does not name any provider.
        contributions.record_discovery_failure("The host could not list providers.");
        assert!(!contributions.has_providers());

        contributions.record_failure("io.github.lithe-idea.maven-build", "The call failed.");
        assert!(contributions.has_providers());

        contributions.forget("io.github.lithe-idea.maven-build");
        assert!(!contributions.has_providers());
        contributions.record_settings(
            "io.github.lithe-idea.gradle-build",
            accepted(json!({ "java.import.gradle.offline.enabled": true })),
        );
        assert!(contributions.has_providers());
    }

    #[test]
    fn forgetting_a_provider_drops_its_settings_and_its_failure() {
        let mut contributions = Contributions::default();
        contributions.record_settings(
            "io.github.lithe-idea.gradle-build",
            accepted(json!({ "java.import.gradle.offline.enabled": true })),
        );
        contributions.record_failure("io.github.lithe-idea.maven-build", "The call failed.");
        contributions.forget("io.github.lithe-idea.gradle-build");
        contributions.forget("io.github.lithe-idea.maven-build");
        assert!(contributions.fragments().is_empty());
        assert_eq!(contributions.status(), ContributedSettingsStatus::Settled);
    }
}
