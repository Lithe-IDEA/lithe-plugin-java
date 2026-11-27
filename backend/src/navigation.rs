//! Bidirectional inheritance/implementation navigation actions for Java files.
//!
//! Registers an `editor/registerActions` collector for `.java` files when JDT LS
//! is active. Syntax candidates identify types and methods whose relation must
//! still be confirmed semantically by JDT LS (`textDocument/implementation`,
//! `java/findLinks`, `textDocument/definition`) before a gutter icon is shown.

use crate::run::{file_uri, Report};
use crate::settings_session::{SettingsSession, REQUEST_METHOD, SERVER_ID};
use lithe_java_domain::lsp::languages::java_navigation_syntax::{
    navigation_candidates, JavaNavigationCandidate, JavaNavigationCandidateKind,
};
use lithe_java_domain::lsp::languages::jdt_navigation::{
    extract_normalized_locations, NormalizedNavigationLocation,
};
use lithe_plugin_sdk::{
    BackendHandle, Context, FunctionError, Host, PluginError, SdkError, Subscription, Value,
    WorkspaceContext,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Registration ID of the Java navigation gutter actions binding per workspace.
pub const ACTIONS_BINDING_ID: &str = "java.navigation";

/// Function the host calls on document idle to collect navigation gutter actions.
pub const COLLECT_FUNCTION_ID: &str = "java.navigation.collect";

/// Function the host calls when the user clicks a navigation gutter icon.
pub const INVOKE_FUNCTION_ID: &str = "java.navigation.invoke";

const LIST_CONTEXTS_METHOD: &str = "lithe/host/workspace/listContexts";
const GET_CAPABILITIES_METHOD: &str = "lithe/host/getCapabilities";
const LANGUAGE_STATUS_METHOD: &str = "language/status";
const SERVICE_READY_TYPE: &str = "ServiceReady";

const SYNC_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const SYNC_RETRY_ATTEMPTS: u32 = 40;
const COLLECT_BUDGET: Duration = Duration::from_millis(4_500);
const COLLECT_LSP_TIMEOUT_MS: u64 = 1_500;
const INVOKE_LSP_TIMEOUT_MS: u64 = 5_000;
const MAX_COLLECT_CANDIDATES: usize = 40;
const MAX_ACTIONS: usize = 1000;

/// Plugin error code for malformed navigation function arguments.
pub const NAV_INVALID_PARAMS: &str = "nav.invalid_params";

/// Plugin error code when a navigation function is called without a workspace context.
pub const NAV_CONTEXT_MISSING: &str = "nav.context_missing";

/// Plugin error code when a workspace has no accessible local folder.
pub const NAV_WORKSPACE_UNAVAILABLE: &str = "nav.workspace_unavailable";

/// Plugin error code when JDT LS rejects or fails a click-triggered navigation query.
pub const NAV_LANGUAGE_SERVER_FAILED: &str = "nav.language_server_failed";

/// Every plugin error code the navigation functions answer with.
pub const NAV_ERROR_CODES: &[&str] = &[
    NAV_INVALID_PARAMS,
    NAV_CONTEXT_MISSING,
    NAV_WORKSPACE_UNAVAILABLE,
    NAV_LANGUAGE_SERVER_FAILED,
];

/// Category reported when registering the navigation actions binding failed.
pub const NAV_ACTIONS_REGISTRATION_FAILED: &str = "nav.actions_registration_failed";

/// Category reported when workspace contexts could not be listed for navigation sync.
pub const NAV_CONTEXTS_UNAVAILABLE: &str = "nav.contexts_unavailable";

/// Every degraded category this module reports.
pub const NAV_CATEGORIES: &[&str] = &[NAV_ACTIONS_REGISTRATION_FAILED, NAV_CONTEXTS_UNAVAILABLE];

struct Registration {
    generation: u64,
    subscription: Subscription,
}

/// Host icon support negotiated once from `lithe/host/getCapabilities`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ActionIconPalette {
    pub semantic: bool,
}

impl ActionIconPalette {
    /// Inspects `services.editor.options.actionIcons` for the four dedicated
    /// IDEA-style navigation tokens (`navigate-implemented`,
    /// `navigate-implementing`, `navigate-overridden`, `navigate-overriding`),
    /// falling back to `navigate-implementations` / `navigate-super` when absent.
    pub fn from_capabilities(capabilities: &Value) -> Self {
        let Some(icons) = capabilities
            .pointer("/services/editor/options/actionIcons")
            .and_then(Value::as_array)
        else {
            return Self { semantic: false };
        };
        let has = |token: &str| icons.iter().any(|entry| entry.as_str() == Some(token));
        let semantic = has("navigate-implemented")
            && has("navigate-implementing")
            && has("navigate-overridden")
            && has("navigate-overriding");
        Self { semantic }
    }

    /// Selects the host-supported icon token for `(direction, relation)`.
    pub fn icon(self, direction: &str, relation: &str) -> &'static str {
        if self.semantic {
            match (direction, relation) {
                ("down", "interface") => "navigate-implemented",
                ("up", "interface") => "navigate-implementing",
                ("down", _) => "navigate-overridden",
                _ => "navigate-overriding",
            }
        } else if direction == "down" {
            "navigate-implementations"
        } else {
            "navigate-super"
        }
    }
}

/// Coordinates editor-action registrations for Java implementation/super navigation.
pub struct NavigationSession {
    handle: BackendHandle,
    settings: Arc<SettingsSession>,
    registrations: Mutex<HashMap<String, Registration>>,
    palette: Mutex<Option<ActionIconPalette>>,
}

impl NavigationSession {
    pub fn new(handle: BackendHandle, settings: Arc<SettingsSession>) -> Self {
        Self {
            handle,
            settings,
            registrations: Mutex::new(HashMap::new()),
            palette: Mutex::new(None),
        }
    }

    fn icon_palette(&self, host: &Host) -> ActionIconPalette {
        if let Some(cached) = *lock(&self.palette) {
            return cached;
        }
        match host.request(GET_CAPABILITIES_METHOD, json!({})) {
            Ok(capabilities) => {
                let palette = ActionIconPalette::from_capabilities(&capabilities);
                *lock(&self.palette) = Some(palette);
                palette
            }
            Err(_) => ActionIconPalette { semantic: false },
        }
    }

    /// Brings active workspace context bindings in line with running JDT LS instances.
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

    pub fn sync_contexts_reporting(&self, report: &dyn Fn(&str, u64)) -> bool {
        match self.sync_contexts() {
            Ok(0) => true,
            Ok(failures) => {
                report(NAV_ACTIONS_REGISTRATION_FAILED, failures);
                true
            }
            Err(_) => {
                report(NAV_CONTEXTS_UNAVAILABLE, 1);
                false
            }
        }
    }

    /// Performs initial binding sync once initialize completes.
    pub fn sync_after_initialize(self: &Arc<Self>, report: Report) {
        let session = Arc::clone(self);
        thread::Builder::new()
            .name("java-nav-sync".into())
            .spawn(move || {
                for attempt in 0..SYNC_RETRY_ATTEMPTS {
                    thread::sleep(SYNC_RETRY_INTERVAL);
                    match session.sync_contexts() {
                        Ok(0) => return,
                        Ok(failures) => {
                            report(NAV_ACTIONS_REGISTRATION_FAILED, failures);
                            return;
                        }
                        Err(_) if attempt + 1 < SYNC_RETRY_ATTEMPTS => continue,
                        Err(_) => report(NAV_CONTEXTS_UNAVAILABLE, 1),
                    }
                }
            })
            .ok();
    }

    /// Refreshes bindings when JDT LS finishes ServiceReady or imports modules.
    pub fn did_receive_notification(&self, params: &Value, report: &dyn Fn(&str, u64)) {
        if !is_refresh_notification(params) {
            return;
        }
        let Ok(reference) = WorkspaceContext::from_value(&params["context"]) else {
            return;
        };
        let Some(generation) = params.get("generation").and_then(Value::as_u64) else {
            return;
        };
        if !self
            .settings
            .accepts_server_notification(&reference, generation)
        {
            return;
        }
        if let Some(stale) = lock(&self.registrations).remove(reference.context_id()) {
            let _ = stale.subscription.dispose();
        }
        self.sync_contexts_reporting(report);
    }
}

/// Collects navigation actions for one `.java` document without blocking indefinitely.
pub fn collect_actions_function(
    session: &NavigationSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let reference = workspace_context(context)?;
    if !session.settings.server_is_running(reference) {
        return Ok(json!({ "actions": [] }));
    }
    let path = params
        .pointer("/document/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| failure(NAV_INVALID_PARAMS, "The document has no path."))?;
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if text.is_empty() {
        return Ok(json!({ "actions": [] }));
    }

    let root = workspace_root(context.host(), reference)?;
    let doc_uri = file_uri(&root.join(path))?;
    let mut candidates = navigation_candidates(text);
    if candidates.is_empty() {
        return Ok(json!({ "actions": [] }));
    }
    // Verify parent (`up`) candidates ahead of child (`down`) candidates when
    // capped, so child classes keep override arrows even in huge files.
    candidates.sort_by_key(|candidate| (candidate.direction != "up", candidate.line));
    candidates.truncate(MAX_COLLECT_CANDIDATES);

    let host = context.host();
    let palette = session.icon_palette(host);
    let started = Instant::now();
    let mut resolved = Vec::new();

    for candidate in candidates {
        context.check_cancelled()?;
        if started.elapsed() >= COLLECT_BUDGET {
            break;
        }
        match resolve_candidate_targets(
            host,
            reference,
            &doc_uri,
            &candidate,
            COLLECT_LSP_TIMEOUT_MS,
        ) {
            Ok(targets) => {
                if !targets.is_empty() {
                    resolved.push(ResolvedCandidate { candidate, targets });
                }
            }
            Err(error) if is_transient_code(error.code()) => {
                return Ok(json!({
                    "actions": build_navigation_actions(&doc_uri, resolved, palette)
                }));
            }
            Err(_) => continue,
        }
    }

    Ok(json!({
        "actions": build_navigation_actions(&doc_uri, resolved, palette)
    }))
}

/// Resolves fresh target locations when the user clicks a navigation gutter icon.
pub fn invoke_action_function(
    _session: &NavigationSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let reference = workspace_context(context)?;
    let data = params
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| failure(NAV_INVALID_PARAMS, "The click carries no navigation data."))?;
    let target = InvokeRequest::from_value(data)?;
    let host = context.host();
    let locations = query_invoke_locations(host, reference, &target, INVOKE_LSP_TIMEOUT_MS)
        .map_err(|error| failure(NAV_LANGUAGE_SERVER_FAILED, error.message().to_owned()))?;
    let reveal_locations: Vec<Value> = locations
        .iter()
        .map(NormalizedNavigationLocation::to_lsp_location_value)
        .collect();
    Ok(json!({
        "reveal": {
            "locations": reveal_locations
        }
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvokeRequest {
    uri: String,
    line: i64,
    utf16_column: i64,
    kind: JavaNavigationCandidateKind,
    direction: String,
    super_positions: Vec<(i64, i64)>,
}

impl InvokeRequest {
    fn from_value(value: &Value) -> Result<Self, FunctionError> {
        let uri = value
            .get("uri")
            .and_then(Value::as_str)
            .filter(|uri| !uri.is_empty())
            .ok_or_else(|| failure(NAV_INVALID_PARAMS, "The action names no document URI."))?
            .to_owned();
        let line = value
            .get("line")
            .and_then(Value::as_i64)
            .ok_or_else(|| failure(NAV_INVALID_PARAMS, "The action names no line."))?;
        let utf16_column = value
            .get("utf16Column")
            .and_then(Value::as_i64)
            .ok_or_else(|| failure(NAV_INVALID_PARAMS, "The action names no column."))?;
        let kind = match value.get("kind").and_then(Value::as_str) {
            Some("type") => JavaNavigationCandidateKind::Type,
            Some("method") => JavaNavigationCandidateKind::Method,
            _ => {
                return Err(failure(
                    NAV_INVALID_PARAMS,
                    "The action names no declaration kind.",
                ))
            }
        };
        let direction = match value.get("direction").and_then(Value::as_str) {
            Some(dir @ ("up" | "down")) => dir.to_owned(),
            _ => {
                return Err(failure(
                    NAV_INVALID_PARAMS,
                    "The action names no direction.",
                ))
            }
        };
        let super_positions = value
            .get("superPositions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|pair| {
                        let arr = pair.as_array()?;
                        Some((arr.first()?.as_i64()?, arr.get(1)?.as_i64()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            uri,
            line,
            utf16_column,
            kind,
            direction,
            super_positions,
        })
    }
}

fn query_invoke_locations(
    host: &Host,
    reference: &WorkspaceContext,
    target: &InvokeRequest,
    timeout_milliseconds: u64,
) -> Result<Vec<NormalizedNavigationLocation>, SdkError> {
    let candidate = JavaNavigationCandidate {
        line: target.line,
        utf16_column: target.utf16_column,
        direction: if target.direction == "down" {
            "down"
        } else {
            "up"
        },
        relation: "inheritance",
        kind: target.kind,
        super_positions: target.super_positions.clone(),
    };
    resolve_candidate_targets(
        host,
        reference,
        &target.uri,
        &candidate,
        timeout_milliseconds,
    )
}

fn resolve_candidate_targets(
    host: &Host,
    reference: &WorkspaceContext,
    doc_uri: &str,
    candidate: &JavaNavigationCandidate,
    timeout_milliseconds: u64,
) -> Result<Vec<NormalizedNavigationLocation>, SdkError> {
    let raw_responses = if candidate.direction == "down" {
        vec![lsp_request(
            host,
            reference,
            "textDocument/implementation",
            json!({
                "textDocument": { "uri": doc_uri },
                "position": { "line": candidate.line, "character": candidate.utf16_column }
            }),
            timeout_milliseconds,
        )?]
    } else if candidate.kind == JavaNavigationCandidateKind::Type
        && !candidate.super_positions.is_empty()
    {
        let mut responses = Vec::with_capacity(candidate.super_positions.len());
        for &(super_line, super_col) in &candidate.super_positions {
            responses.push(lsp_request(
                host,
                reference,
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": doc_uri },
                    "position": { "line": super_line, "character": super_col }
                }),
                timeout_milliseconds,
            )?);
        }
        responses
    } else {
        vec![lsp_request(
            host,
            reference,
            "java/findLinks",
            json!({
                "type": "superImplementation",
                "position": {
                    "textDocument": { "uri": doc_uri },
                    "position": { "line": candidate.line, "character": candidate.utf16_column }
                }
            }),
            timeout_milliseconds,
        )?]
    };

    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for value in raw_responses {
        for loc in extract_normalized_locations(&value) {
            if is_self_location(&loc, doc_uri, candidate.line) {
                continue;
            }
            let key = (loc.uri.clone(), loc.start_line, loc.start_character);
            if seen.insert(key) {
                targets.push(loc);
            }
        }
    }
    Ok(targets)
}

fn is_self_location(
    loc: &NormalizedNavigationLocation,
    doc_uri: &str,
    candidate_line: i64,
) -> bool {
    loc.uri == doc_uri && i64::from(loc.start_line) == candidate_line
}

/// One resolved syntax candidate paired with verified JDT LS target locations.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedCandidate {
    pub candidate: JavaNavigationCandidate,
    pub targets: Vec<NormalizedNavigationLocation>,
}

struct MergedAction {
    line: i64,
    utf16_column: i64,
    direction: &'static str,
    relation: &'static str,
    kind: JavaNavigationCandidateKind,
    super_positions: Vec<(i64, i64)>,
    targets: Vec<NormalizedNavigationLocation>,
}

/// Converts resolved candidates into contract-compliant editor actions,
/// merging multiple super clauses on the same `(line, direction)` into a single
/// gutter lane icon while keeping `up` and `down` in separate lanes.
pub fn build_navigation_actions(
    doc_uri: &str,
    resolved: Vec<ResolvedCandidate>,
    palette: ActionIconPalette,
) -> Vec<Value> {
    let mut merged: BTreeMap<(i64, &'static str), MergedAction> = BTreeMap::new();
    for entry in resolved {
        let targets: Vec<_> = entry
            .targets
            .into_iter()
            .filter(|loc| !is_self_location(loc, doc_uri, entry.candidate.line))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let key = (entry.candidate.line, entry.candidate.direction);
        let slot = merged.entry(key).or_insert_with(|| MergedAction {
            line: entry.candidate.line,
            utf16_column: entry.candidate.utf16_column,
            direction: entry.candidate.direction,
            relation: entry.candidate.relation,
            kind: entry.candidate.kind,
            super_positions: Vec::new(),
            targets: Vec::new(),
        });
        if entry.candidate.relation == "interface" {
            slot.relation = "interface";
        }
        for pos in entry.candidate.super_positions {
            if !slot.super_positions.contains(&pos) {
                slot.super_positions.push(pos);
            }
        }
        for target in targets {
            let dup = slot.targets.iter().any(|existing| {
                existing.uri == target.uri
                    && existing.start_line == target.start_line
                    && existing.start_character == target.start_character
            });
            if !dup {
                slot.targets.push(target);
            }
        }
    }

    merged
        .into_values()
        .take(MAX_ACTIONS)
        .map(|item| {
            let title =
                format_action_title(item.direction, item.relation, item.kind, &item.targets);
            let icon = palette.icon(item.direction, item.relation);
            let line_one_based = item.line + 1;
            let mut invoke_params = json!({
                "uri": doc_uri,
                "line": item.line,
                "utf16Column": item.utf16_column,
                "kind": match item.kind {
                    JavaNavigationCandidateKind::Type => "type",
                    JavaNavigationCandidateKind::Method => "method",
                },
                "direction": item.direction,
            });
            if !item.super_positions.is_empty() {
                invoke_params["superPositions"] = json!(item.super_positions);
            }
            json!({
                "id": format!("nav-{}-{}", item.direction, item.line),
                "range": {
                    "start": { "line": line_one_based, "column": 1 },
                    "end": { "line": line_one_based, "column": 1 }
                },
                "title": title,
                "icon": icon,
                "invoke": {
                    "functionID": INVOKE_FUNCTION_ID,
                    "params": invoke_params,
                }
            })
        })
        .collect()
}

fn format_action_title(
    direction: &str,
    relation: &str,
    kind: JavaNavigationCandidateKind,
    targets: &[NormalizedNavigationLocation],
) -> String {
    let mut labels = Vec::new();
    for target in targets {
        if let Some(label) = target.short_label() {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
    }
    let summary = match labels.len() {
        0 => String::new(),
        1..=3 => labels.join(", "),
        n => format!("{} (+{})", labels[..2].join(", "), n - 2),
    };
    let verb = match (direction, relation, kind) {
        ("down", "interface", _) => "Implemented by",
        ("down", _, _) => "Overridden by",
        ("up", "interface", _) => "Implements",
        ("up", _, JavaNavigationCandidateKind::Type) => "Extends",
        _ => "Overrides",
    };
    if summary.is_empty() {
        format!("{verb} ({})", targets.len())
    } else {
        format!("{verb} {summary}")
    }
}

fn lsp_request(
    host: &Host,
    reference: &WorkspaceContext,
    method: &str,
    params: Value,
    timeout_milliseconds: u64,
) -> Result<Value, SdkError> {
    let answer = host.request(
        REQUEST_METHOD,
        json!({
            "context": reference.to_value(),
            "serverID": SERVER_ID,
            "method": method,
            "params": params,
            "timeoutMilliseconds": timeout_milliseconds,
        }),
    )?;
    Ok(answer.get("value").cloned().unwrap_or(Value::Null))
}

fn is_transient_code(code: &str) -> bool {
    matches!(
        code,
        "plugin.host.languageServerUnavailable"
            | "plugin.host.busy"
            | "plugin.host.timeout"
            | "plugin.host.cancelled"
            | "plugin.host.languageServerError"
    )
}

fn is_refresh_notification(params: &Value) -> bool {
    if params.get("serverID").and_then(Value::as_str) != Some(SERVER_ID) {
        return false;
    }
    match params.get("method").and_then(Value::as_str) {
        Some(LANGUAGE_STATUS_METHOD) => {
            params.pointer("/params/type").and_then(Value::as_str) == Some(SERVICE_READY_TYPE)
        }
        Some("language/eventNotification") => {
            let event = &params["params"]["eventType"];
            matches!(
                event.as_str(),
                Some("ClasspathUpdated" | "ProjectsImported" | "ProjectsDeleted")
            ) || matches!(event.as_u64(), Some(100 | 200 | 210))
        }
        _ => false,
    }
}

fn workspace_context(context: &Context) -> Result<&WorkspaceContext, FunctionError> {
    context
        .workspace_context()
        .ok_or_else(|| failure(NAV_CONTEXT_MISSING, "The call names no workspace."))
}

fn workspace_root(host: &Host, reference: &WorkspaceContext) -> Result<PathBuf, FunctionError> {
    let context = host.workspace().get_context(reference)?;
    context
        .pointer("/folders/0/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            failure(
                NAV_WORKSPACE_UNAVAILABLE,
                "The workspace has no local folder.",
            )
        })
}

fn failure(code: &'static str, message: impl Into<String>) -> FunctionError {
    PluginError::new(code, message).into()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_negotiation_picks_semantic_or_legacy_icons() {
        let modern = ActionIconPalette::from_capabilities(&json!({
            "services": {
                "editor": {
                    "options": {
                        "actionIcons": [
                            "run",
                            "debug",
                            "navigate-super",
                            "navigate-implementations",
                            "navigate-implemented",
                            "navigate-implementing",
                            "navigate-overridden",
                            "navigate-overriding"
                        ]
                    }
                }
            }
        }));
        assert!(modern.semantic);
        assert_eq!(modern.icon("down", "interface"), "navigate-implemented");
        assert_eq!(modern.icon("up", "interface"), "navigate-implementing");
        assert_eq!(modern.icon("down", "inheritance"), "navigate-overridden");
        assert_eq!(modern.icon("up", "inheritance"), "navigate-overriding");

        let legacy = ActionIconPalette::from_capabilities(&json!({
            "services": { "editor": { "options": { "actionIcons": ["navigate-super", "navigate-implementations"] } } }
        }));
        assert!(!legacy.semantic);
        assert_eq!(legacy.icon("down", "interface"), "navigate-implementations");
        assert_eq!(legacy.icon("up", "inheritance"), "navigate-super");
    }

    #[test]
    fn build_navigation_actions_keeps_dual_lanes_and_merges_class_supers() {
        let doc_uri = "file:///workspace/CustomGreeter.java";
        let resolved = vec![
            ResolvedCandidate {
                candidate: JavaNavigationCandidate {
                    line: 10,
                    utf16_column: 6,
                    direction: "up",
                    relation: "inheritance",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: vec![(10, 28)],
                },
                targets: vec![NormalizedNavigationLocation {
                    uri: "file:///workspace/BaseGreeter.java".into(),
                    start_line: 0,
                    start_character: 6,
                    end_line: 0,
                    end_character: 17,
                    display_name: None,
                }],
            },
            ResolvedCandidate {
                candidate: JavaNavigationCandidate {
                    line: 10,
                    utf16_column: 6,
                    direction: "up",
                    relation: "interface",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: vec![(10, 51)],
                },
                targets: vec![NormalizedNavigationLocation {
                    uri: "file:///workspace/Greeter.java".into(),
                    start_line: 0,
                    start_character: 10,
                    end_line: 0,
                    end_character: 17,
                    display_name: None,
                }],
            },
            ResolvedCandidate {
                candidate: JavaNavigationCandidate {
                    line: 12,
                    utf16_column: 18,
                    direction: "up",
                    relation: "interface",
                    kind: JavaNavigationCandidateKind::Method,
                    super_positions: Vec::new(),
                },
                targets: vec![NormalizedNavigationLocation {
                    uri: "file:///workspace/BaseGreeter.java".into(),
                    start_line: 2,
                    start_character: 18,
                    end_line: 2,
                    end_character: 23,
                    display_name: Some("BaseGreeter.greet()".into()),
                }],
            },
            ResolvedCandidate {
                candidate: JavaNavigationCandidate {
                    line: 12,
                    utf16_column: 18,
                    direction: "down",
                    relation: "inheritance",
                    kind: JavaNavigationCandidateKind::Method,
                    super_positions: Vec::new(),
                },
                targets: vec![
                    NormalizedNavigationLocation {
                        uri: doc_uri.into(),
                        start_line: 12,
                        start_character: 18,
                        end_line: 12,
                        end_character: 23,
                        display_name: None,
                    },
                    NormalizedNavigationLocation {
                        uri: "file:///workspace/FormalGreeter.java".into(),
                        start_line: 5,
                        start_character: 18,
                        end_line: 5,
                        end_character: 23,
                        display_name: None,
                    },
                ],
            },
        ];

        let actions =
            build_navigation_actions(doc_uri, resolved, ActionIconPalette { semantic: true });

        assert_eq!(actions.len(), 3);
        // Line 10 class gets single merged up action preferring `navigate-implementing` and carrying both super positions.
        assert_eq!(actions[0]["icon"], "navigate-implementing");
        assert_eq!(actions[0]["range"]["start"]["line"], 11);
        assert_eq!(actions[0]["title"], "Implements BaseGreeter, Greeter");
        assert_eq!(
            actions[0]["invoke"]["params"]["superPositions"],
            json!([[10, 28], [10, 51]])
        );
        // Line 12 method keeps separate down and up icons (BTreeMap sorts "down" before "up").
        assert_eq!(actions[1]["icon"], "navigate-overridden");
        assert_eq!(actions[1]["title"], "Overridden by FormalGreeter");
        assert_eq!(actions[2]["icon"], "navigate-implementing");
        assert_eq!(actions[2]["title"], "Implements BaseGreeter.greet()");
    }

    #[test]
    fn title_formatting_caps_labels_and_rejects_malformed_invoke_params() {
        let targets: Vec<_> = (1..=4)
            .map(|idx| NormalizedNavigationLocation {
                uri: format!("file:///workspace/Impl{idx}.java"),
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 5,
                display_name: None,
            })
            .collect();
        let summary = format_action_title(
            "down",
            "interface",
            JavaNavigationCandidateKind::Type,
            &targets,
        );
        assert_eq!(summary, "Implemented by Impl1, Impl2 (+2)");

        assert!(InvokeRequest::from_value(&json!({})).is_err());
        assert!(InvokeRequest::from_value(&json!({
            "uri": "file:///workspace/A.java",
            "line": 1,
            "utf16Column": 2,
            "kind": "unknown",
            "direction": "down"
        }))
        .is_err());
    }
}
