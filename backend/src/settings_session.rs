//! The settings other plugins contribute to JDT LS, kept per workspace and
//! delivered to the running language server (API-35 on top of API-25).
//!
//! Two things feed one [`Contributions`] per workspace:
//!
//! - When the host reports that the `java` server is `running`, the backend
//!   asks every enabled provider of `languageImport.settings` for its
//!   settings ([`collect_contributions`]).
//! - When a build-system plugin changes its settings, it calls the function
//!   this plugin exports, `languageImport.settingsChanged`
//!   ([`SettingsSession::settings_changed`]). The call carries the complete
//!   new settings, and the host writes the caller's plugin ID, so the call
//!   replaces exactly the caller's slot. This plugin never calls the caller
//!   back: the host judges that a call cycle.
//!
//! Either way the merged result goes to JDT LS as one
//! `workspace/didChangeConfiguration`. That notification replaces the whole
//! plugin configuration of the session, so the backend always sends the
//! complete merged settings and never a delta.
//!
//! # Every running server gets a push
//!
//! The backend resolves the JDT LS launch itself ([`crate::launch`]), so no
//! host integration pushes JDT LS settings any more, and this module is their
//! only source. Once the collection of a running server finished, the backend
//! pushes even when no Maven or Gradle plugin took part: the push then
//! carries the plugin's own settings and the defaults.
//!
//! The launch answer's `workspaceConfiguration` and every push are built by
//! the same function, [`java_configuration`], from the contributions known
//! at that moment, so the settings JDT LS starts with and the settings a
//! later `workspace/didChangeConfiguration` carries never disagree in shape.
//!
//! # Ordering and locks
//!
//! The SDK runs every notification handler on a thread of its own, so the
//! state notifications of one server can be handled out of order. Each event
//! is ordered by `(generation, phase)`, where the phase is `starting`,
//! `running` or `ended` (`stopped` and `crashed`). An event that is not
//! greater than the last applied one is stale or a duplicate and is ignored.
//!
//! The state lock is never held across a host call. Pushes are serialized by
//! a separate `gate` lock, and each push takes its snapshot of the state
//! after it entered the gate, so a push that started earlier can never arrive
//! at the server after a push with newer settings. The lock order is always
//! `gate`, then the state lock.

use crate::plugin_functions::{
    collect_contributions, FunctionRelay, HostRelay, RelayFailure, PROVIDER_TIMEOUT,
    RELAY_KEYS_REJECTED,
};
use lithe_java_domain::lsp::languages::jdt::{settings_notification, JdtJavaRuntime, JdtSettings};
use lithe_java_domain::lsp::languages::jdt_contributions::{
    accept_answer, AcceptedSettings, Contributions, SETTINGS_CHANGED_FUNCTION,
};
use lithe_plugin_sdk::{BackendHandle, Cancellation, PluginError, Value, WorkspaceContext};
use serde_json::{json, Map};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Host notification that reports the state of a plugin-declared language
/// server: `{context, serverID, state, generation}`.
pub const LANGUAGE_SERVER_STATE_NOTIFICATION: &str = "lithe/host/languageServer/didChangeState";

/// The exported function a build-system plugin calls when its settings change.
/// The ID comes from the interface definition so it is written down once.
pub const SETTINGS_CHANGED_FUNCTION_ID: &str = SETTINGS_CHANGED_FUNCTION.id;

/// `languageID` of the language server this plugin declares.
pub const SERVER_ID: &str = "java";

/// The JDT LS command that applies per-project settings.
pub const UPDATE_PROJECT_SETTINGS_COMMAND: &str = "java.project.updateSettings";

/// The project setting, in the `java.project.updateSettings` command, that
/// holds the selected Maven profiles as one comma-separated string.
pub const SELECTED_PROFILES_KEY: &str = "org.eclipse.m2e.core.selectedProfiles";

/// Plugin error code of a `settingsChanged` call that no plugin made through
/// the host's relay, so there is no caller whose slot it could replace.
pub const RELAY_CALLER_MISSING: &str = "relay.caller_missing";

/// Plugin error code of a `settingsChanged` call without a workspace
/// context, so there is no workspace whose settings it could change.
pub const RELAY_CONTEXT_MISSING: &str = "relay.context_missing";

/// Plugin error code of a `settingsChanged` call whose params do not have the
/// documented shape.
pub const RELAY_INVALID_PARAMS: &str = "relay.invalid_params";

/// Every plugin error code `settingsChanged` answers with.
pub const CHANGE_ERROR_CODES: &[&str] = &[
    RELAY_CALLER_MISSING,
    RELAY_CONTEXT_MISSING,
    RELAY_INVALID_PARAMS,
];

/// Category reported when the host could not tell where the workspace is, so
/// the providers were not asked.
pub const SESSION_ROOT_UNAVAILABLE: &str = "session.root_unavailable";

/// Category reported when the merged settings could not be sent to JDT LS.
pub const SESSION_PUSH_FAILED: &str = "session.push_failed";

/// Category reported when updating one project in JDT LS failed.
pub const SESSION_PROJECT_UPDATE_FAILED: &str = "session.project_update_failed";

/// Category reported when a server state notification was malformed.
pub const SESSION_STATE_INVALID: &str = "session.state_invalid";

/// Every category this module reports. Each one is a controlled code that
/// names no plugin, path or message.
pub const SESSION_CATEGORIES: &[&str] = &[
    SESSION_ROOT_UNAVAILABLE,
    SESSION_PUSH_FAILED,
    SESSION_PROJECT_UPDATE_FAILED,
    SESSION_STATE_INVALID,
];

/// Host method that sends a notification to a plugin-declared server.
const NOTIFY_METHOD: &str = "lithe/host/languageServer/notify";

/// Host method that sends a request to a plugin-declared server.
pub(crate) const REQUEST_METHOD: &str = "lithe/host/languageServer/request";

/// LSP notification that replaces the plugin configuration of the session.
const DID_CHANGE_CONFIGURATION: &str = "workspace/didChangeConfiguration";

/// LSP request that runs a server command.
pub(crate) const EXECUTE_COMMAND: &str = "workspace/executeCommand";

/// What the session needs from the host, behind a trait so tests can script
/// it. Every failure is the host's answer to one request.
pub trait SessionHost: Send + Sync {
    /// The local path of the workspace's first folder, or `None` when the
    /// workspace has no local path (a remote workspace).
    fn workspace_root(&self, workspace: &WorkspaceContext) -> Result<Option<String>, RelayFailure>;

    /// Sends `workspace/didChangeConfiguration` with `params` to the server.
    fn push_configuration(
        &self,
        workspace: &WorkspaceContext,
        params: Value,
    ) -> Result<(), RelayFailure>;

    /// Sends `workspace/executeCommand` with `params` to the server and waits
    /// for its answer.
    fn execute_command(
        &self,
        workspace: &WorkspaceContext,
        params: Value,
    ) -> Result<(), RelayFailure>;

    /// The cross-plugin relay for `workspace`, used outside any function call.
    fn relay(&self, workspace: &WorkspaceContext) -> Box<dyn FunctionRelay>;
}

/// The real host, reached through the backend handle so that the work of one
/// notification does not belong to any function call.
pub struct HostServices {
    handle: BackendHandle,
}

impl HostServices {
    pub fn new(handle: BackendHandle) -> Self {
        Self { handle }
    }
}

impl SessionHost for HostServices {
    fn workspace_root(&self, workspace: &WorkspaceContext) -> Result<Option<String>, RelayFailure> {
        let context = self.handle.host().workspace().get_context(workspace)?;
        Ok(first_folder_path(&context))
    }

    fn push_configuration(
        &self,
        workspace: &WorkspaceContext,
        params: Value,
    ) -> Result<(), RelayFailure> {
        self.handle.host().request(
            NOTIFY_METHOD,
            json!({
                "context": workspace.to_value(),
                "serverID": SERVER_ID,
                "method": DID_CHANGE_CONFIGURATION,
                "params": params,
            }),
        )?;
        Ok(())
    }

    fn execute_command(
        &self,
        workspace: &WorkspaceContext,
        params: Value,
    ) -> Result<(), RelayFailure> {
        self.handle.host().request(
            REQUEST_METHOD,
            json!({
                "context": workspace.to_value(),
                "serverID": SERVER_ID,
                "method": EXECUTE_COMMAND,
                "params": params,
            }),
        )?;
        Ok(())
    }

    fn relay(&self, workspace: &WorkspaceContext) -> Box<dyn FunctionRelay> {
        Box::new(HostRelay::new(
            self.handle.host(),
            workspace.clone(),
            PROVIDER_TIMEOUT,
        ))
    }
}

/// The local path of the first folder in a `getContext` answer. A remote
/// workspace's folders carry no `path`.
fn first_folder_path(context: &Value) -> Option<String> {
    context
        .get("folders")?
        .as_array()?
        .first()?
        .get("path")?
        .as_str()
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

/// Where a server process stands. The derived order is the order in which a
/// process moves through the phases, which is what makes events comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ServerPhase {
    Starting,
    Running,
    /// Stopped or crashed.
    Ended,
}

/// The Maven profiles selected for one project, from a `settingsChanged` call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectProfiles {
    project_uri: String,
    selected_profiles: Vec<String>,
}

impl ProjectProfiles {
    /// The params of the `workspace/executeCommand` that applies the profiles.
    /// JDT LS takes them as one comma-separated string, which is empty when
    /// no profile is selected.
    fn command_params(&self) -> Value {
        let mut settings = Map::new();
        settings.insert(
            SELECTED_PROFILES_KEY.to_string(),
            Value::String(self.selected_profiles.join(",")),
        );
        json!({
            "command": UPDATE_PROJECT_SETTINGS_COMMAND,
            "arguments": [self.project_uri, Value::Object(settings)],
        })
    }
}

/// Changes a provider made while its workspace's collection was in flight.
type LateChanges = Vec<(String, AcceptedSettings)>;

/// Everything the session knows about one workspace.
struct WorkspaceState {
    /// The newest context reference seen for the workspace. Requests with an
    /// older generation fail with `plugin.host.contextExpired`.
    context: WorkspaceContext,
    /// The last applied server event, which orders the next one.
    seen: Option<(u64, ServerPhase)>,
    contributions: Contributions,
    /// The JDKs the last launch configured for the server, from the tools
    /// the host resolved; every push repeats them so the server keeps the
    /// same default runtime the host runs.
    java_runtimes: Vec<JdtJavaRuntime>,
    /// Project updates waiting for a running server.
    pending_projects: Vec<ProjectProfiles>,
    /// Set from the moment the server is running until its collection is
    /// finished. It holds the changes that arrive meanwhile: the collection
    /// asked the providers before such a change, so its answer would
    /// otherwise overwrite the change.
    collecting: Option<LateChanges>,
}

impl WorkspaceState {
    fn new(context: WorkspaceContext) -> Self {
        Self {
            context,
            seen: None,
            contributions: Contributions::default(),
            java_runtimes: Vec::new(),
            pending_projects: Vec::new(),
            collecting: None,
        }
    }

    fn is_running(&self) -> bool {
        matches!(self.seen, Some((_, ServerPhase::Running)))
    }

    /// Remembers `context` when it is not older than the one held.
    fn remember_context(&mut self, context: &WorkspaceContext) {
        if context.generation() >= self.context.generation() {
            self.context = context.clone();
        }
    }

    /// Applies a server event unless it is stale or repeated. Returns whether
    /// it was applied.
    fn advance(&mut self, context: &WorkspaceContext, generation: u64, phase: ServerPhase) -> bool {
        let event = (generation, phase);
        if self.seen.is_some_and(|seen| event <= seen) {
            return false;
        }
        self.seen = Some(event);
        self.remember_context(context);
        self.collecting = (phase == ServerPhase::Running).then(Vec::new);
        true
    }
}

/// Queues a project update, replacing an earlier one for the same project.
fn stash_project(pending: &mut Vec<ProjectProfiles>, project: ProjectProfiles) {
    match pending
        .iter_mut()
        .find(|entry| entry.project_uri == project.project_uri)
    {
        Some(entry) => *entry = project,
        None => pending.push(project),
    }
}

/// A server state notification after validation.
enum StateEvent {
    /// About another plugin's server, or a state this version does not know.
    Ignored,
    /// Not shaped like the documented notification.
    Invalid,
    Change {
        workspace: WorkspaceContext,
        generation: u64,
        phase: ServerPhase,
    },
}

fn parse_state_event(params: &Value) -> StateEvent {
    match params.get("serverID").and_then(Value::as_str) {
        Some(SERVER_ID) => {}
        Some(_) => return StateEvent::Ignored,
        None => return StateEvent::Invalid,
    }
    let Ok(workspace) = WorkspaceContext::from_value(&params["context"]) else {
        return StateEvent::Invalid;
    };
    let Some(generation) = params
        .get("generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
    else {
        return StateEvent::Invalid;
    };
    let phase = match params.get("state").and_then(Value::as_str) {
        Some("starting") => ServerPhase::Starting,
        Some("running") => ServerPhase::Running,
        Some("stopped" | "crashed") => ServerPhase::Ended,
        Some(_) => return StateEvent::Ignored,
        None => return StateEvent::Invalid,
    };
    StateEvent::Change {
        workspace,
        generation,
        phase,
    }
}

/// A `settingsChanged` call after validation.
struct SettingsChange {
    accepted: AcceptedSettings,
    projects: Vec<ProjectProfiles>,
}

/// Validates the params of `languageImport.settingsChanged`:
/// `{settings: object, projects?: [{projectUri, selectedProfiles: [string]}]}`.
///
/// Fields the contract does not define are refused instead of ignored: a
/// change that carries more than this version understands would otherwise be
/// applied only in part, and a new field means a new contract version. The
/// error is a sentence without any value from the call.
fn parse_change_params(params: &Value) -> Result<SettingsChange, String> {
    let fields = params
        .as_object()
        .ok_or_else(|| "The params are not an object.".to_string())?;
    if fields
        .keys()
        .any(|key| key != "settings" && key != "projects")
    {
        return Err("The params have a field this version does not define.".to_string());
    }
    let accepted = accept_answer(params)?;
    let projects = match fields.get("projects") {
        None => Vec::new(),
        Some(projects) => parse_projects(projects)?,
    };
    Ok(SettingsChange { accepted, projects })
}

/// Parses `projects`, keeping the first entry of every project URI.
fn parse_projects(projects: &Value) -> Result<Vec<ProjectProfiles>, String> {
    let entries = projects
        .as_array()
        .ok_or_else(|| "`projects` is not an array.".to_string())?;
    let mut parsed: Vec<ProjectProfiles> = Vec::new();
    for entry in entries {
        let project = parse_project(entry)?;
        if !parsed
            .iter()
            .any(|kept| kept.project_uri == project.project_uri)
        {
            parsed.push(project);
        }
    }
    Ok(parsed)
}

fn parse_project(entry: &Value) -> Result<ProjectProfiles, String> {
    let fields = entry
        .as_object()
        .ok_or_else(|| "A project entry is not an object.".to_string())?;
    if fields
        .keys()
        .any(|key| key != "projectUri" && key != "selectedProfiles")
    {
        return Err("A project entry has a field this version does not define.".to_string());
    }
    let project_uri = fields
        .get("projectUri")
        .and_then(Value::as_str)
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| "A project entry has no `projectUri` text.".to_string())?;
    let profiles = fields
        .get("selectedProfiles")
        .and_then(Value::as_array)
        .ok_or_else(|| "A project entry has no `selectedProfiles` array.".to_string())?;
    // A comma separates the profiles in the command, so a profile that
    // contains one would silently turn into two.
    let selected_profiles = profiles
        .iter()
        .map(|profile| {
            profile
                .as_str()
                .filter(|profile| !profile.is_empty() && !profile.contains(','))
                .map(str::to_string)
                .ok_or_else(|| {
                    "A selected profile is not text, is empty or contains a comma.".to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProjectProfiles {
        project_uri: project_uri.to_string(),
        selected_profiles,
    })
}

/// The JDT LS settings this plugin owns, built from the contributed
/// `fragments` and the `runtimes` the launch configured. The launch passes
/// them to `initializationOptions`, and every push sends them, so both start
/// from this one function.
pub fn jdt_settings<'a>(fragments: &'a [Value], runtimes: &'a [JdtJavaRuntime]) -> JdtSettings<'a> {
    JdtSettings {
        // The host overlays its own indexing policy on the plugin
        // configuration it forwards, so the plugin must not compute one.
        indexing_root_uri: None,
        // Plugin-declared completion options and import exclusions belong to
        // the session's initialization, which this module does not see; the
        // notification carries their defaults.
        completion: None,
        import_exclusions: None,
        contributed: fragments,
        // The host sends the installation it resolved for the `java` tool
        // with the launch (`tools.java`); the launch turns it into the
        // default runtime so the server compiles for the JDK that runs the
        // project. Without it the key is absent and JDT LS keeps the VMs it
        // already knows, so the runtimes stay as the previous launch set them.
        java_runtimes: runtimes,
    }
}

/// The params of the `workspace/didChangeConfiguration` for `fragments`:
/// `{settings: {java: ...}}`.
fn configuration_params(fragments: &[Value], runtimes: &[JdtJavaRuntime]) -> Value {
    settings_notification(jdt_settings(fragments, runtimes)).params
}

/// The `settings` member of the push params, `{java: ...}`. The launch
/// answers it as `workspaceConfiguration`; Core serves the server's
/// `workspace/configuration` requests from it and sends it after
/// `initialized`, which is exactly what a later push replaces it with.
pub fn java_configuration(fragments: &[Value], runtimes: &[JdtJavaRuntime]) -> Value {
    match configuration_params(fragments, runtimes) {
        Value::Object(mut params) => params.remove("settings").unwrap_or_else(|| json!({})),
        _ => json!({}),
    }
}

/// Locks `mutex` even when a panic poisoned it: the guarded data stays
/// consistent because every critical section here only swaps whole values.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The contributions and project updates of every workspace, and their
/// delivery to JDT LS.
pub struct SettingsSession {
    host: Arc<dyn SessionHost>,
    /// Keyed by `contextID`.
    states: Mutex<HashMap<String, WorkspaceState>>,
    /// Held while a push and its project updates are delivered; see the
    /// module documentation.
    gate: Mutex<()>,
}

impl SettingsSession {
    pub fn new(host: Arc<dyn SessionHost>) -> Self {
        Self {
            host,
            states: Mutex::new(HashMap::new()),
            gate: Mutex::new(()),
        }
    }

    /// Action collection must not call JDT before its authoritative running event.
    pub(crate) fn server_is_running(&self, context: &WorkspaceContext) -> bool {
        self.lock_states()
            .get(context.context_id())
            .is_some_and(|state| {
                state.context.generation() == context.generation() && state.is_running()
            })
    }

    fn lock_states(&self) -> MutexGuard<'_, HashMap<String, WorkspaceState>> {
        lock(&self.states)
    }

    /// Handles `lithe/host/languageServer/didChangeState`.
    ///
    /// When the `java` server is running, this collects the providers'
    /// settings and pushes the merged result, which is the plugin's own
    /// settings when no provider took part.
    /// The call blocks on the host, which is allowed on a notification
    /// thread. `report` receives the categories in [`SESSION_CATEGORIES`] and
    /// [`crate::plugin_functions::RELAY_CATEGORIES`].
    pub fn did_change_state(&self, params: &Value, report: &dyn Fn(&str, u64)) {
        let (workspace, generation, phase) = match parse_state_event(params) {
            StateEvent::Ignored => return,
            StateEvent::Invalid => {
                report(SESSION_STATE_INVALID, 1);
                return;
            }
            StateEvent::Change {
                workspace,
                generation,
                phase,
            } => (workspace, generation, phase),
        };
        let key = workspace.context_id().to_string();
        {
            let mut states = self.lock_states();
            let state = states
                .entry(key.clone())
                .or_insert_with(|| WorkspaceState::new(workspace.clone()));
            if !state.advance(&workspace, generation, phase) {
                return;
            }
        }
        // A stopped or crashed server keeps its contributions: the next
        // `running` collects again and replaces them.
        if phase != ServerPhase::Running {
            return;
        }
        let collected = self.collect(&workspace, report);
        if self.finish_collection(&key, generation, collected) {
            self.flush(&key, report);
        }
    }

    /// Asks the providers for their settings. `None` when there is nothing to
    /// collect: a remote workspace, or a workspace whose location the host
    /// would not tell. The push still follows, with the contributions known
    /// before.
    fn collect(
        &self,
        workspace: &WorkspaceContext,
        report: &dyn Fn(&str, u64),
    ) -> Option<Contributions> {
        let root = match self.host.workspace_root(workspace) {
            Ok(Some(root)) => root,
            // A remote workspace has no local folder, and the providers can
            // only describe local paths, so they are not asked.
            Ok(None) => return None,
            Err(_) => {
                report(SESSION_ROOT_UNAVAILABLE, 1);
                return None;
            }
        };
        let relay = self.host.relay(workspace);
        // The collection belongs to no function call, so nothing cancels it;
        // a provider that does not answer ends at the relay's own timeout.
        collect_contributions(&*relay, &root, &Cancellation::new(), report).ok()
    }

    /// Stores the collected contributions, unless the server moved on while
    /// they were collected. Returns whether the collection is still current,
    /// which is when the caller should push.
    fn finish_collection(
        &self,
        key: &str,
        generation: u64,
        collected: Option<Contributions>,
    ) -> bool {
        let mut states = self.lock_states();
        let Some(state) = states.get_mut(key) else {
            return false;
        };
        // Only the event that started this collection may finish it. A later
        // event made the answers stale, and its own collection (if any) owns
        // the `collecting` list now.
        if state.seen != Some((generation, ServerPhase::Running)) {
            return false;
        }
        let late = state.collecting.take().unwrap_or_default();
        if let Some(mut contributions) = collected {
            for (provider, accepted) in late {
                contributions.record_settings(&provider, accepted);
            }
            state.contributions = contributions;
        }
        true
    }

    /// Handles a call of `languageImport.settingsChanged`.
    ///
    /// `caller` and `workspace` come from the host, not from `params`.
    /// The call replaces the caller's slot, pushes the merged settings when
    /// the server is running, and then applies the project updates. While the
    /// server is not running the settings stay recorded and the project
    /// updates wait for the next push. Delivery failures are reported, not
    /// returned: the caller cannot act on them, and the next change or
    /// restart sends the state again. The answer is `{}`.
    pub fn settings_changed(
        &self,
        caller: Option<&str>,
        workspace: Option<&WorkspaceContext>,
        params: &Value,
        report: &dyn Fn(&str, u64),
    ) -> Result<Value, PluginError> {
        let caller = caller.ok_or_else(|| {
            PluginError::new(
                RELAY_CALLER_MISSING,
                "The call did not come from another plugin.",
            )
        })?;
        let workspace = workspace.ok_or_else(|| {
            PluginError::new(
                RELAY_CONTEXT_MISSING,
                "The call carries no workspace context.",
            )
        })?;
        let change = parse_change_params(params)
            .map_err(|reason| PluginError::new(RELAY_INVALID_PARAMS, reason))?;
        let rejected = u64::try_from(change.accepted.rejected.len()).unwrap_or(u64::MAX);
        if rejected > 0 {
            report(RELAY_KEYS_REJECTED, rejected);
        }
        let key = workspace.context_id().to_string();
        {
            let mut states = self.lock_states();
            let state = states
                .entry(key.clone())
                .or_insert_with(|| WorkspaceState::new(workspace.clone()));
            state.remember_context(workspace);
            state
                .contributions
                .record_settings(caller, change.accepted.clone());
            if let Some(late) = state.collecting.as_mut() {
                late.push((caller.to_string(), change.accepted));
            }
            for project in change.projects {
                stash_project(&mut state.pending_projects, project);
            }
        }
        self.flush(&key, report);
        Ok(json!({}))
    }

    /// Sends the merged settings and the pending project updates, if the
    /// workspace is ready for them.
    ///
    /// Nothing is sent while the server is not running or while the
    /// collection is still in flight (a push then would carry a partial
    /// result). Whatever is not sent stays recorded for a later flush. A
    /// workspace without providers is pushed as well; see the module
    /// documentation.
    fn flush(&self, key: &str, report: &dyn Fn(&str, u64)) {
        let _delivery = lock(&self.gate);
        let (workspace, params, projects) = {
            let mut states = self.lock_states();
            let Some(state) = states.get_mut(key) else {
                return;
            };
            if !state.is_running() || state.collecting.is_some() {
                return;
            }
            (
                state.context.clone(),
                configuration_params(&state.contributions.fragments(), &state.java_runtimes),
                std::mem::take(&mut state.pending_projects),
            )
        };
        if self.host.push_configuration(&workspace, params).is_err() {
            report(SESSION_PUSH_FAILED, 1);
            // The server did not get the settings the updates build on, so
            // the updates wait for the next successful push.
            self.requeue_projects(key, projects);
            return;
        }
        for project in &projects {
            if self
                .host
                .execute_command(&workspace, project.command_params())
                .is_err()
            {
                report(SESSION_PROJECT_UPDATE_FAILED, 1);
            }
        }
    }

    /// The contributed settings fragments known for the workspace
    /// `context_id`, in the order a push applies them; empty for a workspace
    /// the session has not seen. The launch builds its startup settings from
    /// them, so a server that restarts starts with the last pushed settings.
    pub fn start_fragments(&self, context_id: &str) -> Vec<Value> {
        self.lock_states()
            .get(context_id)
            .map(|state| state.contributions.fragments())
            .unwrap_or_default()
    }

    /// Remembers the JDKs a launch of the workspace `workspace` configured,
    /// so a later push repeats them instead of leaving the server with the
    /// runtimes of an older launch. A workspace the session has not seen is
    /// created here, as a later state event would create it.
    pub fn set_java_runtimes(&self, workspace: &WorkspaceContext, runtimes: Vec<JdtJavaRuntime>) {
        let mut states = self.lock_states();
        let state = states
            .entry(workspace.context_id().to_string())
            .or_insert_with(|| WorkspaceState::new(workspace.clone()));
        state.java_runtimes = runtimes;
    }

    /// The JDKs the last launch of the workspace `context_id` configured;
    /// empty for a workspace the session has not seen.
    pub fn java_runtimes(&self, context_id: &str) -> Vec<JdtJavaRuntime> {
        self.lock_states()
            .get(context_id)
            .map(|state| state.java_runtimes.clone())
            .unwrap_or_default()
    }

    /// Puts project updates back in front of the ones that arrived while they
    /// were being sent; a newer update of the same project wins.
    fn requeue_projects(&self, key: &str, mut projects: Vec<ProjectProfiles>) {
        if projects.is_empty() {
            return;
        }
        let mut states = self.lock_states();
        if let Some(state) = states.get_mut(key) {
            for newer in std::mem::take(&mut state.pending_projects) {
                stash_project(&mut projects, newer);
            }
            state.pending_projects = projects;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_functions::{Provider, RELAY_CATEGORIES};
    use lithe_java_domain::lsp::languages::jdt_contributions::FunctionContract;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const MAVEN: &str = "io.github.lithe-idea.maven-build";
    const GRADLE: &str = "io.github.lithe-idea.gradle-build";

    /// What the fake host received, in order.
    #[derive(Debug, Clone, PartialEq)]
    enum Sent {
        Configuration(WorkspaceContext, Value),
        Command(WorkspaceContext, Value),
    }

    /// The script a test gives the fake host, and the log it reads back.
    struct Script {
        root: Mutex<Result<Option<String>, RelayFailure>>,
        root_calls: AtomicUsize,
        providers: Mutex<Vec<Provider>>,
        provider_calls: AtomicUsize,
        answers: Mutex<BTreeMap<String, Result<Value, RelayFailure>>>,
        /// Runs once, in the first provider call, after that call's answer
        /// was read. It lets a test change the world mid-collection.
        on_invoke: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        push_failure: Mutex<Option<RelayFailure>>,
        failing_project: Mutex<Option<String>>,
        sent: Mutex<Vec<Sent>>,
    }

    impl Script {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                root: Mutex::new(Ok(Some("/work/app".to_string()))),
                root_calls: AtomicUsize::new(0),
                providers: Mutex::new(Vec::new()),
                provider_calls: AtomicUsize::new(0),
                answers: Mutex::new(BTreeMap::new()),
                on_invoke: Mutex::new(None),
                push_failure: Mutex::new(None),
                failing_project: Mutex::new(None),
                sent: Mutex::new(Vec::new()),
            })
        }

        /// A provider of the interface version this plugin speaks that
        /// answers with `settings`.
        fn provide(&self, plugin: &str, settings: Value) {
            let mut providers = lock(&self.providers);
            if !providers.iter().any(|known| known.plugin_id == plugin) {
                providers.push(Provider {
                    plugin_id: plugin.to_string(),
                    version: 1,
                });
            }
            lock(&self.answers).insert(plugin.to_string(), Ok(json!({ "settings": settings })));
        }

        fn on_invoke(&self, hook: impl FnOnce() + Send + 'static) {
            *lock(&self.on_invoke) = Some(Box::new(hook));
        }

        fn sent(&self) -> Vec<Sent> {
            lock(&self.sent).clone()
        }

        /// The params of every configuration push.
        fn configurations(&self) -> Vec<Value> {
            self.sent()
                .into_iter()
                .filter_map(|sent| match sent {
                    Sent::Configuration(_, params) => Some(params),
                    Sent::Command(..) => None,
                })
                .collect()
        }

        /// The params of every project command.
        fn commands(&self) -> Vec<Value> {
            self.sent()
                .into_iter()
                .filter_map(|sent| match sent {
                    Sent::Command(_, params) => Some(params),
                    Sent::Configuration(..) => None,
                })
                .collect()
        }
    }

    struct FakeHost(Arc<Script>);

    impl SessionHost for FakeHost {
        fn workspace_root(
            &self,
            _workspace: &WorkspaceContext,
        ) -> Result<Option<String>, RelayFailure> {
            self.0.root_calls.fetch_add(1, Ordering::SeqCst);
            lock(&self.0.root).clone()
        }

        fn push_configuration(
            &self,
            workspace: &WorkspaceContext,
            params: Value,
        ) -> Result<(), RelayFailure> {
            if let Some(failure) = lock(&self.0.push_failure).clone() {
                return Err(failure);
            }
            lock(&self.0.sent).push(Sent::Configuration(workspace.clone(), params));
            Ok(())
        }

        fn execute_command(
            &self,
            workspace: &WorkspaceContext,
            params: Value,
        ) -> Result<(), RelayFailure> {
            let failing = lock(&self.0.failing_project).clone();
            if let Some(uri) = failing {
                if params["arguments"][0].as_str() == Some(uri.as_str()) {
                    return Err(RelayFailure::new(
                        "plugin.host.languageServerError",
                        "The command failed.",
                    ));
                }
            }
            lock(&self.0.sent).push(Sent::Command(workspace.clone(), params));
            Ok(())
        }

        fn relay(&self, _workspace: &WorkspaceContext) -> Box<dyn FunctionRelay> {
            Box::new(ScriptedRelay(Arc::clone(&self.0)))
        }
    }

    struct ScriptedRelay(Arc<Script>);

    impl FunctionRelay for ScriptedRelay {
        fn providers(&self, _function: &FunctionContract) -> Result<Vec<Provider>, RelayFailure> {
            self.0.provider_calls.fetch_add(1, Ordering::SeqCst);
            Ok(lock(&self.0.providers).clone())
        }

        fn invoke(
            &self,
            provider: &str,
            _function: &FunctionContract,
            _params: Value,
        ) -> Result<Value, RelayFailure> {
            // The answer is read before the hook runs, so a hook that changes
            // the script cannot change what this call already answered.
            let answer = lock(&self.0.answers)
                .get(provider)
                .cloned()
                .unwrap_or_else(|| Err(RelayFailure::new("plugin.host.notFound", "No provider.")));
            let hook = lock(&self.0.on_invoke).take();
            if let Some(hook) = hook {
                hook();
            }
            answer
        }
    }

    fn fixture() -> (Arc<Script>, Arc<SettingsSession>) {
        let script = Script::new();
        let session = Arc::new(SettingsSession::new(Arc::new(FakeHost(Arc::clone(
            &script,
        )))));
        (script, session)
    }

    fn workspace() -> WorkspaceContext {
        WorkspaceContext::new("ctx-1", 1).expect("a valid context")
    }

    fn state_of(server: &str, state: &str, generation: u64) -> Value {
        json!({
            "context": { "contextID": "ctx-1", "generation": 1 },
            "serverID": server,
            "state": state,
            "generation": generation,
        })
    }

    /// A state notification of the `java` server.
    fn java(state: &str, generation: u64) -> Value {
        state_of(SERVER_ID, state, generation)
    }

    fn quiet(_: &str, _: u64) {}

    fn change(
        session: &SettingsSession,
        caller: &str,
        params: Value,
    ) -> Result<Value, PluginError> {
        session.settings_changed(Some(caller), Some(&workspace()), &params, &quiet)
    }

    fn enabled(value: bool) -> Value {
        json!({ "settings": { "java.import.maven.enabled": value } })
    }

    #[test]
    fn actions_wait_for_running_and_ignore_late_events_after_stop() {
        let (_script, session) = fixture();
        assert!(!session.server_is_running(&workspace()));
        session.did_change_state(&java("starting", 1), &quiet);
        assert!(!session.server_is_running(&workspace()));
        session.did_change_state(&java("running", 1), &quiet);
        assert!(session.server_is_running(&workspace()));
        session.did_change_state(&java("stopped", 1), &quiet);
        session.did_change_state(&java("running", 1), &quiet);
        assert!(!session.server_is_running(&workspace()));
        session.did_change_state(&java("running", 2), &quiet);
        assert!(session.server_is_running(&workspace()));
        let replaced = WorkspaceContext::new("ctx-1", 2).unwrap();
        assert!(!session.server_is_running(&replaced));
    }

    #[test]
    fn the_constants_match_the_contract() {
        assert_eq!(
            LANGUAGE_SERVER_STATE_NOTIFICATION,
            "lithe/host/languageServer/didChangeState"
        );
        assert_eq!(
            SETTINGS_CHANGED_FUNCTION_ID,
            "languageImport.settingsChanged"
        );
        assert!(lithe_plugin_sdk::is_function_id(
            SETTINGS_CHANGED_FUNCTION_ID
        ));
        assert_eq!(SERVER_ID, "java");
        assert_eq!(
            UPDATE_PROJECT_SETTINGS_COMMAND,
            "java.project.updateSettings"
        );
        assert_eq!(
            SELECTED_PROFILES_KEY,
            "org.eclipse.m2e.core.selectedProfiles"
        );
    }

    #[test]
    fn no_provider_still_pushes_the_plugins_own_settings() {
        let (script, session) = fixture();
        session.did_change_state(&java("running", 1), &quiet);
        // The providers were asked, and none exists.
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 1);
        // The push is unconditional: it carries the plugin's own settings and
        // the defaults, and no contributed value.
        let configurations = script.configurations();
        assert_eq!(configurations.len(), 1, "{configurations:?}");
        assert_eq!(
            configurations[0].pointer("/settings/java/maven/downloadSources"),
            Some(&json!(false))
        );
        assert!(configurations[0]
            .pointer("/settings/java/import/maven/enabled")
            .is_none());
        assert_eq!(configurations[0], configuration_params(&[], &[]));
    }

    #[test]
    fn the_startup_configuration_is_what_a_push_sends() {
        let (script, session) = fixture();
        // Before the first `running`, the launch sees no contribution.
        assert!(session.start_fragments(workspace().context_id()).is_empty());

        script.provide(
            MAVEN,
            json!({
                "java.import.maven.enabled": true,
                "java.configuration.maven.userSettings": "/m/settings.xml",
            }),
        );
        session.did_change_state(&java("running", 1), &quiet);
        let pushed = script.configurations();
        assert_eq!(pushed.len(), 1);

        // A restart's launch answers the same settings the last push sent.
        let fragments = session.start_fragments(workspace().context_id());
        assert_eq!(fragments.len(), 1);
        assert_eq!(java_configuration(&fragments, &[]), pushed[0]["settings"]);
        assert_eq!(
            java_configuration(&[], &[]),
            configuration_params(&[], &[])["settings"]
        );
        assert!(session.start_fragments("ctx-unknown").is_empty());
    }

    #[test]
    fn a_provider_that_failed_still_counts_as_taking_part() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({}));
        lock(&script.answers).insert(
            MAVEN.to_string(),
            Err(RelayFailure::new("plugin.function.failed", "Broken.")),
        );
        session.did_change_state(&java("running", 1), &quiet);
        // The push carries the defaults only, and replaces nothing the
        // provider could have contributed.
        assert_eq!(script.configurations().len(), 1);
    }

    #[test]
    fn providers_mean_one_push_with_their_fragments() {
        let (script, session) = fixture();
        script.provide(
            MAVEN,
            json!({
                "java.import.maven.enabled": true,
                "java.configuration.maven.userSettings": "/m/settings.xml",
            }),
        );
        script.provide(
            GRADLE,
            json!({ "java.import.gradle.offline.enabled": true }),
        );
        session.did_change_state(&java("running", 1), &quiet);

        let sent = script.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        let Sent::Configuration(context, params) = &sent[0] else {
            panic!("expected a configuration push: {sent:?}");
        };
        assert_eq!(context, &workspace());
        assert_eq!(
            params
                .as_object()
                .map(|fields| fields.keys().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["settings"])
        );
        assert_eq!(
            params.pointer("/settings/java/import/maven/enabled"),
            Some(&json!(true))
        );
        assert_eq!(
            params.pointer("/settings/java/configuration/maven/userSettings"),
            Some(&json!("/m/settings.xml"))
        );
        assert_eq!(
            params.pointer("/settings/java/import/gradle/offline/enabled"),
            Some(&json!(true))
        );
        // The defaults stay, and without a launch that configured runtimes
        // the key is absent so JDT LS keeps the VMs it already knows.
        assert_eq!(
            params.pointer("/settings/java/maven/downloadSources"),
            Some(&json!(false))
        );
        assert!(params
            .pointer("/settings/java/configuration/runtimes")
            .is_none());
    }

    #[test]
    fn a_push_repeats_the_runtimes_the_launch_configured() {
        let (script, session) = fixture();
        let runtimes = vec![JdtJavaRuntime {
            is_default: true,
            name: "JavaSE-17".to_string(),
            path: "/jdk/17".to_string(),
        }];
        // The launch runs before the first `running`.
        session.set_java_runtimes(&workspace(), runtimes.clone());
        assert_eq!(session.java_runtimes(workspace().context_id()), runtimes);
        assert!(session.java_runtimes("ctx-unknown").is_empty());

        session.did_change_state(&java("running", 1), &quiet);
        let pushed = script.configurations();
        assert_eq!(pushed.len(), 1);
        assert_eq!(
            pushed[0].pointer("/settings/java/configuration/runtimes"),
            Some(&json!([{"name": "JavaSE-17", "path": "/jdk/17", "default": true}]))
        );
        assert_eq!(pushed[0], configuration_params(&[], &runtimes));
    }

    #[test]
    fn a_remote_workspace_is_not_collected() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        *lock(&script.root) = Ok(None);
        session.did_change_state(&java("running", 1), &quiet);
        assert_eq!(script.root_calls.load(Ordering::SeqCst), 1);
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 0);
        // The plugin's own settings are still pushed, without a provider's.
        let configurations = script.configurations();
        assert_eq!(configurations.len(), 1, "{configurations:?}");
        assert!(configurations[0]
            .pointer("/settings/java/import/maven/enabled")
            .is_none());
    }

    #[test]
    fn a_server_other_than_java_is_ignored() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        let reports = RefCell::new(Vec::<String>::new());
        let report = |code: &str, _: u64| reports.borrow_mut().push(code.to_string());
        for state in ["starting", "running", "stopped", "crashed", "unheard-of"] {
            session.did_change_state(&state_of("kotlin", state, 1), &report);
        }
        // Even a malformed notification of another server is not an error.
        session.did_change_state(&json!({ "serverID": "kotlin" }), &report);
        assert_eq!(script.root_calls.load(Ordering::SeqCst), 0);
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 0);
        assert!(script.sent().is_empty());
        assert!(reports.borrow().is_empty(), "{:?}", reports.borrow());
    }

    #[test]
    fn a_server_that_stopped_during_collection_gets_no_push() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        let weak = Arc::downgrade(&session);
        script.on_invoke(move || {
            weak.upgrade()
                .expect("the session is alive")
                .did_change_state(&java("stopped", 1), &quiet);
        });
        session.did_change_state(&java("running", 1), &quiet);
        assert!(script.sent().is_empty());

        // The next process collects again and pushes.
        session.did_change_state(&java("running", 2), &quiet);
        assert_eq!(script.configurations().len(), 1);
    }

    #[test]
    fn a_restart_during_collection_discards_the_old_answers() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        let weak = Arc::downgrade(&session);
        let hook_script = Arc::clone(&script);
        script.on_invoke(move || {
            // The provider's settings changed, and the server restarted: the
            // newer collection sees the new value, while the older one is
            // still about to return the old one.
            hook_script.provide(MAVEN, json!({ "java.import.maven.enabled": false }));
            weak.upgrade()
                .expect("the session is alive")
                .did_change_state(&java("running", 2), &quiet);
        });
        session.did_change_state(&java("running", 1), &quiet);

        let configurations = script.configurations();
        assert_eq!(configurations.len(), 1, "{configurations:?}");
        assert_eq!(
            configurations[0].pointer("/settings/java/import/maven/enabled"),
            Some(&json!(false))
        );
    }

    #[test]
    fn a_change_during_collection_is_not_lost_and_waits_for_the_result() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        let weak = Arc::downgrade(&session);
        let hook_script = Arc::clone(&script);
        script.on_invoke(move || {
            let session = weak.upgrade().expect("the session is alive");
            change(&session, MAVEN, enabled(false)).expect("a valid change");
            // No partial push while the collection is in flight.
            assert!(hook_script.sent().is_empty());
        });
        session.did_change_state(&java("running", 1), &quiet);

        let configurations = script.configurations();
        assert_eq!(configurations.len(), 1, "{configurations:?}");
        // The older answer (true) was read before the change (false).
        assert_eq!(
            configurations[0].pointer("/settings/java/import/maven/enabled"),
            Some(&json!(false))
        );
    }

    #[test]
    fn events_are_ordered_by_generation_and_phase() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        // `stopped` overtook its own `running`: the process is already gone.
        session.did_change_state(&java("stopped", 1), &quiet);
        session.did_change_state(&java("running", 1), &quiet);
        session.did_change_state(&java("starting", 1), &quiet);
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 0);

        // A duplicate `running` collects once.
        session.did_change_state(&java("starting", 2), &quiet);
        session.did_change_state(&java("running", 2), &quiet);
        session.did_change_state(&java("running", 2), &quiet);
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(script.configurations().len(), 1);

        // A late event of the replaced process changes nothing.
        session.did_change_state(&java("stopped", 1), &quiet);
        change(&session, MAVEN, enabled(false)).expect("a valid change");
        assert_eq!(script.configurations().len(), 2);
    }

    #[test]
    fn a_stopped_server_keeps_its_settings_until_the_next_running() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);
        session.did_change_state(&java("crashed", 1), &quiet);

        // A change while the server is down is recorded but not sent.
        change(&session, MAVEN, enabled(false)).expect("a valid change");
        assert_eq!(script.configurations().len(), 1);

        // The restart collects the provider's answer again.
        script.provide(MAVEN, json!({ "java.import.maven.enabled": false }));
        session.did_change_state(&java("running", 2), &quiet);
        let configurations = script.configurations();
        assert_eq!(configurations.len(), 2);
        assert_eq!(
            configurations[1].pointer("/settings/java/import/maven/enabled"),
            Some(&json!(false))
        );
    }

    #[test]
    fn a_change_needs_a_caller_and_a_context() {
        let (script, session) = fixture();
        let params = enabled(true);

        let error = session
            .settings_changed(None, Some(&workspace()), &params, &quiet)
            .expect_err("no caller");
        assert_eq!(error.code(), RELAY_CALLER_MISSING);

        let error = session
            .settings_changed(Some(MAVEN), None, &params, &quiet)
            .expect_err("no context");
        assert_eq!(error.code(), RELAY_CONTEXT_MISSING);

        // The caller is checked first.
        let error = session
            .settings_changed(None, None, &params, &quiet)
            .expect_err("neither");
        assert_eq!(error.code(), RELAY_CALLER_MISSING);

        // Neither call changed anything: a running server gets no settings
        // from them.
        script.provide(GRADLE, json!({}));
        session.did_change_state(&java("running", 1), &quiet);
        let pushed = script.configurations();
        assert_eq!(pushed.len(), 1);
        assert!(pushed[0]
            .pointer("/settings/java/import/maven/enabled")
            .is_none());
    }

    #[test]
    fn a_change_replaces_only_the_callers_slot() {
        let (script, session) = fixture();
        script.provide(
            MAVEN,
            json!({
                "java.import.maven.enabled": true,
                "java.configuration.maven.userSettings": "/m/settings.xml",
            }),
        );
        script.provide(
            GRADLE,
            json!({ "java.import.gradle.offline.enabled": true }),
        );
        session.did_change_state(&java("running", 1), &quiet);

        let answer = change(&session, MAVEN, enabled(false)).expect("a valid change");
        assert_eq!(answer, json!({}));

        let configurations = script.configurations();
        assert_eq!(configurations.len(), 2);
        let pushed = &configurations[1];
        assert_eq!(
            pushed.pointer("/settings/java/import/maven/enabled"),
            Some(&json!(false))
        );
        // A setting the caller no longer sends disappears with its old slot.
        assert!(pushed
            .pointer("/settings/java/configuration/maven/userSettings")
            .is_none());
        // The other provider's slot is untouched.
        assert_eq!(
            pushed.pointer("/settings/java/import/gradle/offline/enabled"),
            Some(&json!(true))
        );
    }

    #[test]
    fn a_change_that_is_not_valid_leaves_the_state_alone() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);

        let error = change(&session, MAVEN, json!({ "settings": 5 })).expect_err("invalid");
        assert_eq!(error.code(), RELAY_INVALID_PARAMS);
        assert_eq!(script.configurations().len(), 1);

        change(
            &session,
            GRADLE,
            json!({ "settings": { "java.import.gradle.offline.enabled": true } }),
        )
        .expect("a valid change");
        let configurations = script.configurations();
        assert_eq!(configurations.len(), 2);
        // The failed call did not touch the earlier answer of MAVEN, and the
        // valid call of GRADLE joined it.
        assert_eq!(
            configurations[1].pointer("/settings/java/import/maven/enabled"),
            Some(&json!(true))
        );
        assert_eq!(
            configurations[1].pointer("/settings/java/import/gradle/offline/enabled"),
            Some(&json!(true))
        );
    }

    #[test]
    fn keys_outside_the_namespace_are_dropped_and_reported_by_count() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);

        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let report = |code: &str, count: u64| reports.borrow_mut().push((code.to_string(), count));
        session
            .settings_changed(
                Some(MAVEN),
                Some(&workspace()),
                &json!({ "settings": { "java.import.maven.enabled": true, "java.home": "/x", "other": 1 } }),
                &report,
            )
            .expect("a valid change");
        assert_eq!(*reports.borrow(), [(RELAY_KEYS_REJECTED.to_string(), 2)]);
        let configurations = script.configurations();
        assert!(configurations[1].pointer("/settings/java/home").is_none());
        assert!(configurations[1].pointer("/settings/other").is_none());
    }

    #[test]
    fn projects_wait_for_the_server_and_follow_the_push() {
        let (script, session) = fixture();
        let projects = json!([
            { "projectUri": "file:///w/a", "selectedProfiles": ["dev"] },
            { "projectUri": "file:///w/b", "selectedProfiles": ["ci"] },
        ]);
        change(
            &session,
            MAVEN,
            json!({ "settings": { "java.import.maven.enabled": true }, "projects": projects }),
        )
        .expect("a valid change");
        // The server has not started: nothing is sent.
        assert!(script.sent().is_empty());

        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);

        let sent = script.sent();
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert!(matches!(sent[0], Sent::Configuration(..)));
        assert_eq!(
            sent[1],
            Sent::Command(
                workspace(),
                json!({
                    "command": "java.project.updateSettings",
                    "arguments": [
                        "file:///w/a",
                        { "org.eclipse.m2e.core.selectedProfiles": "dev" }
                    ]
                })
            )
        );
        assert!(
            matches!(&sent[2], Sent::Command(_, params) if params["arguments"][0] == "file:///w/b")
        );

        // They were sent once; the next push does not repeat them.
        change(&session, MAVEN, enabled(false)).expect("a valid change");
        assert_eq!(script.commands().len(), 2);
    }

    #[test]
    fn project_updates_are_deduplicated_and_their_profiles_joined_by_commas() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);

        change(
            &session,
            MAVEN,
            json!({
                "settings": {},
                "projects": [
                    { "projectUri": "file:///w/a", "selectedProfiles": ["dev", "ci"] },
                    { "projectUri": "file:///w/b", "selectedProfiles": [] },
                    { "projectUri": "file:///w/a", "selectedProfiles": ["other"] },
                ]
            }),
        )
        .expect("a valid change");

        assert_eq!(
            script.commands(),
            [
                json!({
                    "command": "java.project.updateSettings",
                    "arguments": [
                        "file:///w/a",
                        { "org.eclipse.m2e.core.selectedProfiles": "dev,ci" }
                    ]
                }),
                json!({
                    "command": "java.project.updateSettings",
                    "arguments": [
                        "file:///w/b",
                        { "org.eclipse.m2e.core.selectedProfiles": "" }
                    ]
                }),
            ]
        );
    }

    #[test]
    fn a_later_call_replaces_a_waiting_update_of_the_same_project() {
        let (script, session) = fixture();
        for profiles in [json!(["dev"]), json!(["ci"])] {
            change(
                &session,
                MAVEN,
                json!({
                    "settings": {},
                    "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": profiles }]
                }),
            )
            .expect("a valid change");
        }
        script.provide(MAVEN, json!({}));
        session.did_change_state(&java("running", 1), &quiet);
        let commands = script.commands();
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(
            commands[0]["arguments"][1],
            json!({ "org.eclipse.m2e.core.selectedProfiles": "ci" })
        );
    }

    #[test]
    fn a_failed_root_lookup_is_reported_and_nothing_is_asked() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        *lock(&script.root) = Err(RelayFailure::new(
            "plugin.host.contextExpired",
            "The context at /work/app is gone.",
        ));
        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let report = |code: &str, count: u64| reports.borrow_mut().push((code.to_string(), count));
        session.did_change_state(&java("running", 1), &report);
        assert_eq!(
            *reports.borrow(),
            [(SESSION_ROOT_UNAVAILABLE.to_string(), 1)]
        );
        assert_eq!(script.provider_calls.load(Ordering::SeqCst), 0);
        // No provider was asked, so the push carries only the plugin's own
        // settings.
        let configurations = script.configurations();
        assert_eq!(configurations.len(), 1, "{configurations:?}");
        assert!(configurations[0]
            .pointer("/settings/java/import/maven/enabled")
            .is_none());
    }

    #[test]
    fn a_failed_push_is_reported_and_keeps_the_project_updates_for_later() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        *lock(&script.push_failure) = Some(RelayFailure::new(
            "plugin.host.languageServerUnavailable",
            "The server is gone.",
        ));
        session.did_change_state(&java("running", 1), &quiet);

        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let report = |code: &str, count: u64| reports.borrow_mut().push((code.to_string(), count));
        session
            .settings_changed(
                Some(MAVEN),
                Some(&workspace()),
                &json!({
                    "settings": { "java.import.maven.enabled": true },
                    "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": ["dev"] }]
                }),
                &report,
            )
            .expect("delivery failures are not the caller's error");
        assert_eq!(*reports.borrow(), [(SESSION_PUSH_FAILED.to_string(), 1)]);
        assert!(script.sent().is_empty());

        // Once the server answers again, the next change delivers both.
        *lock(&script.push_failure) = None;
        change(&session, MAVEN, enabled(true)).expect("a valid change");
        let sent = script.sent();
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(matches!(sent[0], Sent::Configuration(..)));
        assert!(
            matches!(&sent[1], Sent::Command(_, params) if params["arguments"][0] == "file:///w/a")
        );
    }

    #[test]
    fn a_failed_project_update_does_not_stop_the_others() {
        let (script, session) = fixture();
        script.provide(MAVEN, json!({ "java.import.maven.enabled": true }));
        session.did_change_state(&java("running", 1), &quiet);
        *lock(&script.failing_project) = Some("file:///w/a".to_string());

        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let report = |code: &str, count: u64| reports.borrow_mut().push((code.to_string(), count));
        session
            .settings_changed(
                Some(MAVEN),
                Some(&workspace()),
                &json!({
                    "settings": {},
                    "projects": [
                        { "projectUri": "file:///w/a", "selectedProfiles": ["dev"] },
                        { "projectUri": "file:///w/b", "selectedProfiles": ["ci"] },
                    ]
                }),
                &report,
            )
            .expect("delivery failures are not the caller's error");
        assert_eq!(
            *reports.borrow(),
            [(SESSION_PROJECT_UPDATE_FAILED.to_string(), 1)]
        );
        let commands = script.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["arguments"][0], "file:///w/b");
    }

    #[test]
    fn malformed_state_notifications_are_reported_and_unknown_states_are_not() {
        let (script, session) = fixture();
        let reports = RefCell::new(Vec::<(String, u64)>::new());
        let report = |code: &str, count: u64| reports.borrow_mut().push((code.to_string(), count));
        for malformed in [
            json!({}),
            json!({ "serverID": 7 }),
            json!({ "serverID": "java", "state": "running", "generation": 1 }),
            json!({ "serverID": "java", "context": { "contextID": "ctx-1" }, "state": "running", "generation": 1 }),
            json!({ "serverID": "java", "context": { "contextID": "ctx-1", "generation": 1 }, "generation": 1 }),
            json!({ "serverID": "java", "context": { "contextID": "ctx-1", "generation": 1 }, "state": "running" }),
            json!({ "serverID": "java", "context": { "contextID": "ctx-1", "generation": 1 }, "state": "running", "generation": 0 }),
            json!({ "serverID": "java", "context": { "contextID": "ctx-1", "generation": 1 }, "state": "running", "generation": "1" }),
        ] {
            session.did_change_state(&malformed, &report);
        }
        assert_eq!(reports.borrow().len(), 8, "{:?}", reports.borrow());
        assert!(reports
            .borrow()
            .iter()
            .all(|report| report == &(SESSION_STATE_INVALID.to_string(), 1)));

        reports.borrow_mut().clear();
        session.did_change_state(&java("paused", 1), &report);
        assert!(reports.borrow().is_empty());
        assert_eq!(script.root_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn change_params_are_validated_strictly() {
        let valid = parse_change_params(&json!({
            "settings": { "java.import.maven.enabled": true },
            "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": ["dev", "ci"] }]
        }))
        .expect("valid params");
        assert_eq!(
            valid.projects,
            [ProjectProfiles {
                project_uri: "file:///w/a".to_string(),
                selected_profiles: vec!["dev".to_string(), "ci".to_string()],
            }]
        );
        assert!(parse_change_params(&json!({ "settings": {} }))
            .expect("projects are optional")
            .projects
            .is_empty());

        for invalid in [
            json!("settings"),
            json!([]),
            json!({}),
            json!({ "settings": 5 }),
            json!({ "settings": {}, "extra": true }),
            json!({ "settings": {}, "projects": {} }),
            json!({ "settings": {}, "projects": ["file:///w/a"] }),
            json!({ "settings": {}, "projects": [{ "selectedProfiles": [] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "", "selectedProfiles": [] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": 7, "selectedProfiles": [] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a" }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": "dev" }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": [7] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": [""] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": ["a,b"] }] }),
            json!({ "settings": {}, "projects": [{ "projectUri": "file:///w/a", "selectedProfiles": [], "extra": 1 }] }),
        ] {
            assert!(parse_change_params(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn the_first_folder_path_is_the_workspace_root() {
        assert_eq!(
            first_folder_path(&json!({
                "folders": [
                    { "rootID": "r1", "name": "app", "path": "/work/app" },
                    { "rootID": "r2", "name": "lib", "path": "/work/lib" },
                ]
            })),
            Some("/work/app".to_string())
        );
        for no_path in [
            json!({}),
            json!({ "folders": [] }),
            json!({ "folders": "x" }),
            json!({ "folders": [{ "rootID": "r1", "name": "app" }] }),
            json!({ "folders": [{ "rootID": "r1", "path": "" }] }),
            json!({ "folders": [{ "rootID": "r1", "path": 7 }] }),
        ] {
            assert_eq!(first_folder_path(&no_path), None, "{no_path}");
        }
    }

    #[test]
    fn the_codes_and_categories_are_distinct_and_controlled() {
        let mut seen = BTreeSet::new();
        for code in CHANGE_ERROR_CODES {
            assert!(lithe_plugin_sdk::is_plugin_error_code(code), "{code}");
            assert!(seen.insert(*code), "{code} is used twice");
        }
        for category in SESSION_CATEGORIES.iter().chain(RELAY_CATEGORIES) {
            assert!(!category.is_empty() && category.len() <= 64, "{category}");
            assert!(
                category.bytes().all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-' | b'$')),
                "{category}"
            );
            // A plugin error code and a diagnostics category never share a
            // name, so a log fact identifies one cause.
            assert!(seen.insert(*category), "{category} is used twice");
        }
        assert_eq!(
            seen.len(),
            CHANGE_ERROR_CODES.len() + SESSION_CATEGORIES.len() + RELAY_CATEGORIES.len()
        );
    }
}
