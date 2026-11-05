//! Java test discovery and debug workflow rules.
//!
//! Ported from the Swift `JavaTestWorkflowState` and
//! `JavaTestDebugWorkflowCoordinator`. The host keeps the tasks and the editor;
//! this module decides which selection can be debugged, which operation is
//! still current, and how per-file discovery results become Tests tree rows.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::jdt_test_items::{LanguageTestItem, LanguageTestItemKind};

/// Selection the user asked to debug.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestScope {
    /// The whole workspace.
    Workspace,
    /// Every test in one source file.
    File {
        /// Absolute source path.
        path: String,
    },
    /// One discovered class or method.
    TestCase {
        /// Identifier forwarded unchanged to the Java Test extension.
        identifier: String,
        /// Source file of the test, when known.
        #[serde(default)]
        path: Option<String>,
    },
}

/// Resolved Java test target passed into debug preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaTestDebugRequest {
    /// Lexically normalized source file.
    pub file_path: PathBuf,
    /// Selected test, or `None` to debug the whole file.
    pub test_identifier: Option<String>,
}

/// Accepts a debug selection or explains why it cannot be debugged.
pub fn resolve_debug_request(
    provider_id: &str,
    scope: &TestScope,
) -> Result<JavaTestDebugRequest, String> {
    if provider_id != "java" {
        return Err("Java test debugging is currently available for Java projects only".into());
    }
    match scope {
        TestScope::Workspace => Err("Select a Java test file or test case to debug".into()),
        TestScope::File { path } => Ok(JavaTestDebugRequest {
            file_path: normalize(Path::new(path)),
            test_identifier: None,
        }),
        TestScope::TestCase { identifier, path } => {
            let Some(path) = path else {
                return Err("The selected Java test has no source file".into());
            };
            Ok(JavaTestDebugRequest {
                file_path: normalize(Path::new(path)),
                test_identifier: Some(identifier.clone()),
            })
        }
    }
}

/// Removes `.` components and folds `..` into its parent without touching
/// the file system, like Foundation's `standardizedFileURL`.
pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// Discovery answer for one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDiscovery {
    /// The file's test rows, possibly empty.
    Items(Vec<LanguageTestItem>),
    /// Discovery failed; the file row is kept so the user can still run it.
    Failed,
}

/// Why a discovery projection stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Superseded;

/// Expands the file rows of `base_items` with their discovered tests.
///
/// Workspace rows are kept. A file stays when discovery found tests or failed;
/// a file without tests is dropped. When every file answered and none has
/// tests, the result is empty so the Tests view shows its empty state.
/// `is_current` is checked before each file and once at the end; a stale
/// operation stops with [`Superseded`].
pub fn project_discovered_items(
    base_items: &[LanguageTestItem],
    mut discover: impl FnMut(&str) -> Result<FileDiscovery, Superseded>,
    is_current: impl Fn() -> bool,
) -> Result<Vec<LanguageTestItem>, Superseded> {
    let files = base_items
        .iter()
        .filter(|item| item.kind == LanguageTestItemKind::File && item.file_path.is_some())
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(base_items.to_vec());
    }
    let mut projected = base_items
        .iter()
        .filter(|item| item.kind == LanguageTestItemKind::Workspace)
        .cloned()
        .collect::<Vec<_>>();
    let mut completed_file_count = 0;
    for file in files {
        if !is_current() {
            return Err(Superseded);
        }
        let path = file.file_path.as_deref().unwrap_or_default();
        match discover(path)? {
            FileDiscovery::Items(details) => {
                completed_file_count += 1;
                if !details.is_empty() {
                    projected.push(file.clone());
                    projected.extend(details);
                }
            }
            FileDiscovery::Failed => projected.push(file.clone()),
        }
    }
    if !is_current() {
        return Err(Superseded);
    }
    if completed_file_count > 0
        && projected
            .iter()
            .all(|item| item.kind == LanguageTestItemKind::Workspace)
    {
        return Ok(Vec::new());
    }
    Ok(projected)
}

/// Identifier of one run preparation, discovery or debug launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationToken(u64);

/// One kind of cancellable operation where only the latest may finish.
///
/// Beginning a new operation revokes the previous one; finishing or cancelling
/// clears the slot so a late completion of a revoked operation is ignored.
#[derive(Debug, Default)]
pub struct OperationSlot {
    next: u64,
    current: Option<OperationToken>,
}

impl OperationSlot {
    /// Revokes the current operation and returns the token of a new one.
    pub fn begin(&mut self) -> OperationToken {
        self.next += 1;
        let token = OperationToken(self.next);
        self.current = Some(token);
        token
    }

    /// Whether `token` is still the operation allowed to apply its result.
    pub fn is_current(&self, token: OperationToken) -> bool {
        self.current == Some(token)
    }

    /// Clears the slot when `token` is current. Returns whether it was.
    pub fn finish(&mut self, token: OperationToken) -> bool {
        if self.current != Some(token) {
            return false;
        }
        self.current = None;
        true
    }

    /// Revokes whatever operation is current.
    pub fn cancel(&mut self) {
        self.current = None;
    }
}

/// Test workflow state shared by discovery, run preparation and debugging.
///
/// `S` is the result-channel handle of the debug launch. It is stopped
/// whenever the launch that owns it is cancelled, replaced or ends.
#[derive(Debug)]
pub struct JavaTestWorkflowState<S: StopHandle> {
    /// Run preparation (build before a test run).
    pub run_preparation: OperationSlot,
    /// Per-file test discovery for the Tests view.
    pub discovery: OperationSlot,
    debug_launch: OperationSlot,
    result_server: Option<S>,
}

/// Something that releases a resource when stopped.
pub trait StopHandle {
    /// Releases the resource. Stopping twice must be harmless.
    fn stop(&mut self);
}

impl<S: StopHandle> Default for JavaTestWorkflowState<S> {
    fn default() -> Self {
        Self {
            run_preparation: OperationSlot::default(),
            discovery: OperationSlot::default(),
            debug_launch: OperationSlot::default(),
            result_server: None,
        }
    }
}

impl<S: StopHandle> JavaTestWorkflowState<S> {
    /// Cancels any running debug launch and begins a new one.
    pub fn begin_debug_launch(&mut self) -> OperationToken {
        self.cancel_debug_launch();
        self.debug_launch.begin()
    }

    /// Whether `token` is the debug launch that may still start a session.
    pub fn is_current_debug_launch(&self, token: OperationToken) -> bool {
        self.debug_launch.is_current(token)
    }

    /// Cancels the debug launch and releases its result channel.
    pub fn cancel_debug_launch(&mut self) {
        self.debug_launch.cancel();
        self.stop_result_server();
    }

    /// Ends `token`'s launch; optionally releases the result channel.
    pub fn finish_debug_launch(&mut self, token: OperationToken, stop_result_server: bool) {
        if self.debug_launch.finish(token) && stop_result_server {
            self.stop_result_server();
        }
    }

    /// Adopts the result channel of a prepared launch, releasing any older one.
    pub fn adopt_result_server(&mut self, server: S) {
        self.stop_result_server();
        self.result_server = Some(server);
    }

    /// Whether a result channel is held.
    pub fn has_result_server(&self) -> bool {
        self.result_server.is_some()
    }

    /// Releases the held result channel, for example when debugging stops.
    pub fn stop_result_server(&mut self) {
        if let Some(mut server) = self.result_server.take() {
            server.stop();
        }
    }
}

/// What the host must do after a prepared debug launch returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugLaunchStep {
    /// The launch was superseded; the prepared result channel was stopped.
    Discarded,
    /// Start the debug session with the adopted result channel.
    StartSession,
}

/// Applies a prepared launch to the workflow state.
///
/// A prepared launch whose operation is no longer current releases its own
/// result channel instead of replacing the current one.
pub fn adopt_prepared_launch<S: StopHandle>(
    state: &mut JavaTestWorkflowState<S>,
    token: OperationToken,
    mut prepared_server: S,
) -> DebugLaunchStep {
    if !state.is_current_debug_launch(token) {
        prepared_server.stop();
        return DebugLaunchStep::Discarded;
    }
    state.adopt_result_server(prepared_server);
    DebugLaunchStep::StartSession
}

/// Default notification when the generic debugger refuses the session.
pub const DEBUG_START_FAILED: &str = "Could not debug the Java test";

/// Notification shown when saving the test's dirty source file fails.
pub fn save_failed_message(file_name: &str) -> String {
    format!("Could not save {file_name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Debug, Clone, Default)]
    struct CountingServer(Rc<Cell<usize>>);

    impl StopHandle for CountingServer {
        fn stop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn row(kind: LanguageTestItemKind, id: &str, path: Option<&str>) -> LanguageTestItem {
        LanguageTestItem {
            id: id.into(),
            provider_id: "java".into(),
            label: id.into(),
            kind,
            file_path: path.map(Into::into),
            test_identifier: None,
            depth: 0,
            report_identity: None,
            host_fields: Map::new(),
        }
    }

    #[test]
    fn only_java_files_and_located_test_cases_can_be_debugged() {
        assert_eq!(
            resolve_debug_request("python", &TestScope::Workspace).unwrap_err(),
            "Java test debugging is currently available for Java projects only"
        );
        assert_eq!(
            resolve_debug_request("java", &TestScope::Workspace).unwrap_err(),
            "Select a Java test file or test case to debug"
        );
        assert_eq!(
            resolve_debug_request(
                "java",
                &TestScope::TestCase {
                    identifier: "A#b".into(),
                    path: None
                }
            )
            .unwrap_err(),
            "The selected Java test has no source file"
        );
        let file = resolve_debug_request(
            "java",
            &TestScope::File {
                path: "/w/src/./test/../test/A.java".into(),
            },
        )
        .unwrap();
        assert_eq!(file.file_path, PathBuf::from("/w/src/test/A.java"));
        assert_eq!(file.test_identifier, None);
        let case = resolve_debug_request(
            "java",
            &TestScope::TestCase {
                identifier: "A#b".into(),
                path: Some("/w/A.java".into()),
            },
        )
        .unwrap();
        assert_eq!(case.test_identifier.as_deref(), Some("A#b"));
    }

    #[test]
    fn scopes_decode_from_tagged_json() {
        let scope: TestScope =
            serde_json::from_value(json!({"kind": "testCase", "identifier": "A#b"})).unwrap();
        assert_eq!(
            scope,
            TestScope::TestCase {
                identifier: "A#b".into(),
                path: None
            }
        );
    }

    #[test]
    fn stopping_or_replacing_run_preparation_revokes_identical_selections() {
        let mut state = JavaTestWorkflowState::<CountingServer>::default();
        let first = state.run_preparation.begin();
        let next = state.run_preparation.begin();
        assert!(!state.run_preparation.finish(first));
        assert!(!state.run_preparation.is_current(first));
        assert!(state.run_preparation.is_current(next));
        state.run_preparation.cancel();
        assert!(!state.run_preparation.is_current(next));
    }

    #[test]
    fn a_cancelled_debug_launch_cannot_adopt_its_result_server() {
        let mut state = JavaTestWorkflowState::default();
        let token = state.begin_debug_launch();
        state.cancel_debug_launch();
        let stops = Rc::new(Cell::new(0));
        let step = adopt_prepared_launch(&mut state, token, CountingServer(stops.clone()));
        assert_eq!(step, DebugLaunchStep::Discarded);
        assert_eq!(stops.get(), 1);
        assert!(!state.has_result_server());
    }

    #[test]
    fn a_new_debug_launch_releases_the_previous_result_server() {
        let mut state = JavaTestWorkflowState::default();
        let first = state.begin_debug_launch();
        let stops = Rc::new(Cell::new(0));
        assert_eq!(
            adopt_prepared_launch(&mut state, first, CountingServer(stops.clone())),
            DebugLaunchStep::StartSession
        );
        state.finish_debug_launch(first, false);
        assert!(state.has_result_server());
        let _second = state.begin_debug_launch();
        assert_eq!(stops.get(), 1);
        assert!(!state.has_result_server());
    }

    #[test]
    fn finishing_with_a_failure_releases_the_result_server_once() {
        let mut state = JavaTestWorkflowState::default();
        let token = state.begin_debug_launch();
        let stops = Rc::new(Cell::new(0));
        adopt_prepared_launch(&mut state, token, CountingServer(stops.clone()));
        state.finish_debug_launch(token, true);
        state.finish_debug_launch(token, true);
        assert_eq!(stops.get(), 1);
    }

    #[test]
    fn discovery_keeps_files_with_tests_or_errors_and_drops_empty_files() {
        let base = vec![
            row(LanguageTestItemKind::Workspace, "workspace", None),
            row(LanguageTestItemKind::File, "A", Some("/w/A.java")),
            row(LanguageTestItemKind::File, "B", Some("/w/B.java")),
            row(LanguageTestItemKind::File, "C", Some("/w/C.java")),
        ];
        let projected = project_discovered_items(
            &base,
            |path| {
                Ok(match path {
                    "/w/A.java" => FileDiscovery::Items(vec![row(
                        LanguageTestItemKind::TestCase,
                        "A#a",
                        Some(path),
                    )]),
                    "/w/B.java" => FileDiscovery::Items(Vec::new()),
                    _ => FileDiscovery::Failed,
                })
            },
            || true,
        )
        .unwrap();
        let ids = projected
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["workspace", "A", "A#a", "C"]);
    }

    #[test]
    fn discovery_without_any_tests_is_empty() {
        let base = vec![
            row(LanguageTestItemKind::Workspace, "workspace", None),
            row(LanguageTestItemKind::File, "A", Some("/w/A.java")),
        ];
        let projected =
            project_discovered_items(&base, |_| Ok(FileDiscovery::Items(Vec::new())), || true)
                .unwrap();
        assert!(projected.is_empty());
    }

    #[test]
    fn a_superseded_discovery_stops() {
        let base = vec![row(LanguageTestItemKind::File, "A", Some("/w/A.java"))];
        let current = Cell::new(true);
        let result = project_discovered_items(
            &base,
            |_| {
                current.set(false);
                Ok(FileDiscovery::Failed)
            },
            || current.get(),
        );
        assert_eq!(result, Err(Superseded));
    }

    #[test]
    fn rows_without_files_pass_through() {
        let base = vec![row(LanguageTestItemKind::Workspace, "workspace", None)];
        let projected =
            project_discovered_items(&base, |_| unreachable!("no file to discover"), || true)
                .unwrap();
        assert_eq!(projected, base);
    }

    #[test]
    fn notifications_keep_the_host_wording() {
        assert_eq!(save_failed_message("A.java"), "Could not save A.java");
        assert_eq!(DEBUG_START_FAILED, "Could not debug the Java test");
    }
}
