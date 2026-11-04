//! Java Test extension items and the launch plan derived from them.
//!
//! JDT LS's Java Test extension owns the semantic identity of every test class
//! and method. This module turns its answers into the generic Tests tree,
//! build-tool selectors and the Java test debug launch target, without ever
//! deriving an identity from a file name, package declaration or method
//! spelling.
//!
//! Ported from the Java branches of the Swift `LanguageToolingSessionManager`
//! (`discoverJavaTestItems`, `javaBuildTestSelectors`,
//! `resolveJavaTestDebugLaunchTarget`, `javaTestLaunchArguments`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::debug::{JavaTestFramework, JavaTestLaunchRequest};
use crate::protocol::{CoreError, ErrorCode};

/// Java Test extension test level of a test class.
pub const CLASS_LEVEL: i64 = 5;
/// Java Test extension test level of a test method.
pub const METHOD_LEVEL: i64 = 6;
/// Largest number of selectors handed to a build tool in one run.
pub const MAX_BUILD_SELECTORS: usize = 64;
/// JDT LS command that resolves the runner arguments of selected tests.
pub const LAUNCH_ARGUMENTS_COMMAND: &str = "vscode.java.test.junit.argument";
/// Main class of the packaged TestNG runner.
pub const TESTNG_RUNNER_MAIN_CLASS: &str = "com.microsoft.java.test.runner.Launcher";

/// Source range of a test item, in zero-based lines and UTF-16 columns.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaTestRange {
    /// First line of the declaration.
    pub start_line: i64,
    /// UTF-16 column on the first line.
    pub start_utf16_column: i64,
    /// Last line of the declaration.
    pub end_line: i64,
    /// UTF-16 column on the last line.
    pub end_utf16_column: i64,
}

/// One test class or method reported by the Java Test extension.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaTestItem {
    /// Extension identifier, stable within one session.
    pub id: String,
    /// Display label.
    pub label: String,
    /// `Class` or `Class#method(...)` name used by runners and reports.
    pub full_name: String,
    /// JDT project that owns the test.
    pub project_name: String,
    /// Framework kind: 0 JUnit 4, 1 JUnit 5, 2 TestNG.
    #[serde(rename = "testKind")]
    pub kind: i64,
    /// Item level: [`CLASS_LEVEL`] or [`METHOD_LEVEL`].
    #[serde(rename = "testLevel")]
    pub level: i64,
    /// JDT handle identifier, required to launch a single JUnit method.
    #[serde(default)]
    pub jdt_handler: Option<String>,
    /// Sort key chosen by the extension.
    #[serde(default)]
    pub sort_text: Option<String>,
    /// Declaration range in the source file.
    #[serde(default)]
    pub range: Option<JavaTestRange>,
    /// Nested classes and methods.
    #[serde(default)]
    pub children: Vec<JavaTestItem>,
}

impl JavaTestItem {
    /// Whether `identifier` names this item by any identity the host may hold.
    pub fn matches(&self, identifier: &str) -> bool {
        self.id == identifier
            || self.label == identifier
            || self.full_name == identifier
            || self.jdt_handler.as_deref() == Some(identifier)
    }
}

/// Kind of a row in the generic Tests tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageTestItemKind {
    /// The provider's workspace row.
    Workspace,
    /// A source file.
    File,
    /// A class or method inside a file.
    TestCase,
}

/// Canonical report names a test row is associated with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportIdentity {
    /// Suite (class) name in test reports.
    pub suite: String,
    /// Test case name inside the suite, `None` for the whole suite.
    #[serde(default)]
    pub name: Option<String>,
}

/// One row of the generic Tests tree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageTestItem {
    /// Row identifier.
    pub id: String,
    /// Provider that owns the row; Java rows use `"java"`.
    pub provider_id: String,
    /// Display label.
    pub label: String,
    /// Row kind.
    pub kind: LanguageTestItemKind,
    /// Absolute source file path.
    #[serde(default)]
    pub file_path: Option<String>,
    /// Identifier forwarded unchanged to run and debug.
    #[serde(default)]
    pub test_identifier: Option<String>,
    /// Nesting below the provider section; files start at zero.
    pub depth: usize,
    /// Report association used to match results to rows.
    #[serde(default)]
    pub report_identity: Option<ReportIdentity>,
    /// Host-owned fields (project path, build reference, ...) kept verbatim.
    #[serde(flatten)]
    pub host_fields: Map<String, Value>,
}

/// All items depth first, parents before their children.
pub fn flatten(items: &[JavaTestItem]) -> Vec<&JavaTestItem> {
    let mut flattened = Vec::new();
    for item in items {
        flattened.push(item);
        flattened.extend(flatten(&item.children));
    }
    flattened
}

/// Items in the extension's order: sort text, then label, then identifier.
pub fn sorted(items: &[JavaTestItem]) -> Vec<&JavaTestItem> {
    let mut sorted = items.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        let key = |item: &JavaTestItem| {
            (
                item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
                item.label.clone(),
                item.id.clone(),
            )
        };
        key(left).cmp(&key(right))
    });
    sorted
}

/// Tests tree rows for the items of one source file, starting at `depth`.
pub fn project_language_test_items(
    items: &[JavaTestItem],
    file_path: &str,
    depth: usize,
) -> Vec<LanguageTestItem> {
    sorted(items)
        .into_iter()
        .flat_map(|item| {
            std::iter::once(LanguageTestItem {
                id: item.id.clone(),
                provider_id: "java".to_string(),
                label: item.label.clone(),
                kind: LanguageTestItemKind::TestCase,
                file_path: Some(file_path.to_string()),
                test_identifier: Some(item.full_name.clone()),
                depth,
                report_identity: report_identity(item),
                host_fields: Map::new(),
            })
            .chain(project_language_test_items(
                &item.children,
                file_path,
                depth + 1,
            ))
            .collect::<Vec<_>>()
        })
        .collect()
}

/// Report names of one item, following JDT's display-name convention.
///
/// A class reports as its full name; a method `Class#name(params)` reports as
/// suite `Class` and case `name`.
pub fn report_identity(item: &JavaTestItem) -> Option<ReportIdentity> {
    if item.level == CLASS_LEVEL {
        return Some(ReportIdentity {
            suite: item.full_name.clone(),
            name: None,
        });
    }
    if item.level != METHOD_LEVEL {
        return None;
    }
    let (suite, name) = item.full_name.split_once('#')?;
    let name = name.split('(').next().unwrap_or(name);
    Some(ReportIdentity {
        suite: suite.to_string(),
        name: Some(name.to_string()),
    })
}

/// Build-tool selectors for a file or one identified test.
///
/// Values are the extension's full names, unique and sorted. A file yields its
/// top-level items; an identifier must select exactly one item. More than
/// [`MAX_BUILD_SELECTORS`] selectors, or none, is unsupported.
///
/// TODO(host-api: API-35 cross-plugin function): this plugin only produces the
/// selectors. Running them is the job of the build plugin that owns the
/// project (Maven or Gradle). Once the host relays cross-plugin calls, the
/// backend asks the host for the enabled plugins that export the test-run
/// function ID this plugin declares and sends the selectors there; it never
/// names Maven or Gradle. Where the result appears in the native test panel
/// is decided with the editor-action API (API-27). See `MIGRATION.md`.
pub fn build_test_selectors(
    items: &[JavaTestItem],
    identifier: Option<&str>,
) -> Result<Vec<String>, CoreError> {
    let selected = match identifier {
        Some(identifier) => flatten(items)
            .into_iter()
            .filter(|item| item.matches(identifier))
            .collect::<Vec<_>>(),
        None => items.iter().collect(),
    };
    let mut values = selected
        .into_iter()
        .map(|item| item.full_name.clone())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    if values.is_empty()
        || values.len() > MAX_BUILD_SELECTORS
        || (identifier.is_some() && values.len() != 1)
    {
        return Err(unavailable("Java"));
    }
    Ok(values)
}

/// Selected tests and the request that asks JDT LS for their runner arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaTestLaunchPlan {
    /// Session name: the item label for one item, else the file name.
    pub name: String,
    /// JDT project shared by every selected item.
    pub project_name: String,
    /// Framework shared by every selected item.
    pub framework: JavaTestFramework,
    /// Level shared by every selected item.
    pub level: i64,
    /// Framework kind shared by every selected item.
    pub kind: i64,
    /// Names sent in `testNames` of the launch-arguments request.
    pub launch_test_names: Vec<String>,
    /// TestNG method names the runner executes; empty for JUnit.
    pub testng_test_names: Vec<String>,
}

impl JavaTestLaunchPlan {
    /// Single JSON-string argument of [`LAUNCH_ARGUMENTS_COMMAND`], with sorted keys.
    pub fn launch_arguments_request(&self) -> String {
        json!({
            "projectName": self.project_name,
            "testKind": self.kind,
            "testLevel": self.level,
            "testNames": self.launch_test_names,
        })
        .to_string()
    }
}

/// Chooses the tests to debug from one file's items.
///
/// Without an identifier every top-level class is selected. All selected items
/// must share one project, framework kind and level, because one runner
/// process debugs them.
pub fn plan_debug_launch(
    items: &[JavaTestItem],
    file_name: &str,
    test_identifier: Option<&str>,
) -> Result<JavaTestLaunchPlan, CoreError> {
    let identifier = test_identifier.filter(|value| !value.is_empty());
    let selected = match identifier {
        Some(identifier) => flatten(items)
            .into_iter()
            .filter(|item| item.matches(identifier))
            .collect::<Vec<_>>(),
        None => items
            .iter()
            .filter(|item| item.level == CLASS_LEVEL)
            .collect(),
    };
    let Some(first) = selected.first() else {
        return Err(unavailable(&format!(
            "No Java test was found for {}.",
            test_identifier.unwrap_or(file_name)
        )));
    };
    if selected.iter().any(|item| {
        item.project_name != first.project_name
            || item.kind != first.kind
            || item.level != first.level
    }) {
        return Err(unavailable(
            "Debug one Java test framework and project at a time.",
        ));
    }
    let framework = match first.kind {
        0 | 1 => JavaTestFramework::Junit,
        2 => JavaTestFramework::Testng,
        _ => {
            return Err(unavailable(
                "The selected Java test framework is not supported.",
            ))
        }
    };
    let launch_test_names = if framework == JavaTestFramework::Junit && first.level == METHOD_LEVEL
    {
        selected
            .iter()
            .filter_map(|item| item.jdt_handler.clone())
            .collect::<Vec<_>>()
    } else {
        selected.iter().map(|item| item.full_name.clone()).collect()
    };
    if launch_test_names.len() != selected.len() {
        return Err(unavailable(
            "The Java language service returned incomplete test identifiers.",
        ));
    }
    let testng_test_names = if framework == JavaTestFramework::Testng {
        selected
            .iter()
            .flat_map(|item| testng_method_names(item))
            .collect()
    } else {
        Vec::new()
    };
    Ok(JavaTestLaunchPlan {
        name: if selected.len() == 1 {
            first.label.clone()
        } else {
            file_name.to_string()
        },
        project_name: first.project_name.clone(),
        framework,
        level: first.level,
        kind: first.kind,
        launch_test_names,
        testng_test_names,
    })
}

fn testng_method_names(item: &JavaTestItem) -> Vec<String> {
    if item.level == METHOD_LEVEL {
        return vec![item.full_name.clone()];
    }
    item.children.iter().flat_map(testng_method_names).collect()
}

/// Runner arguments JDT LS resolved for a launch plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedJavaTestLaunchArguments {
    /// Runner working directory.
    pub working_directory: String,
    /// JUnit runner main class; absent for TestNG.
    pub main_class: Option<String>,
    /// JDT project name echoed by the extension.
    pub project_name: Option<String>,
    /// Class path entries.
    pub class_paths: Vec<String>,
    /// Module path entries.
    pub module_paths: Vec<String>,
    /// JVM arguments.
    pub vm_arguments: Vec<String>,
    /// Runner program arguments.
    pub program_arguments: Vec<String>,
}

/// Reads the answer of [`LAUNCH_ARGUMENTS_COMMAND`].
pub fn parse_launch_arguments(value: &Value) -> Result<ResolvedJavaTestLaunchArguments, CoreError> {
    let Some(response) = value.as_object() else {
        return Err(unavailable(
            "The Java language service returned invalid test launch arguments.",
        ));
    };
    if let Some(message) = response
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
    {
        return Err(unavailable(message));
    }
    let body = response.get("body").and_then(Value::as_object);
    let Some((body, working_directory)) = body.and_then(|body| {
        body.get("workingDirectory")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|directory| (body, directory.to_string()))
    }) else {
        return Err(unavailable(
            "The Java language service returned incomplete test launch arguments.",
        ));
    };
    let text = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    };
    let strings = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ResolvedJavaTestLaunchArguments {
        working_directory,
        main_class: text("mainClass"),
        project_name: text("projectName"),
        class_paths: strings("classpath"),
        module_paths: strings("modulepath"),
        vm_arguments: strings("vmArguments"),
        program_arguments: strings("programArguments"),
    })
}

/// Everything the debug launch needs about the selected tests.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaTestDebugLaunchTarget {
    /// Source file the tests were selected from.
    pub file_path: String,
    /// Session name.
    pub name: String,
    /// Runner framework.
    pub framework: JavaTestFramework,
    /// Runner working directory.
    pub working_directory: String,
    /// Runner main class.
    pub main_class: String,
    /// JDT project name.
    #[serde(default)]
    pub project_name: Option<String>,
    /// Class path entries.
    #[serde(default)]
    pub class_paths: Vec<String>,
    /// Module path entries.
    #[serde(default)]
    pub module_paths: Vec<String>,
    /// JVM arguments.
    #[serde(default)]
    pub vm_arguments: Vec<String>,
    /// Runner program arguments.
    #[serde(default)]
    pub program_arguments: Vec<String>,
    /// Packaged TestNG runner jar.
    #[serde(default)]
    pub testng_runner_path: Option<String>,
    /// TestNG methods to run.
    #[serde(default)]
    pub testng_test_names: Vec<String>,
}

impl JavaTestDebugLaunchTarget {
    /// Request for [`crate::debug::java_test_launch`] with the result port.
    pub fn launch_request(&self, result_port: u16) -> JavaTestLaunchRequest {
        JavaTestLaunchRequest {
            name: self.name.clone(),
            framework: self.framework,
            working_directory: self.working_directory.clone(),
            main_class: self.main_class.clone(),
            project_name: self.project_name.clone(),
            class_paths: self.class_paths.clone(),
            module_paths: self.module_paths.clone(),
            vm_arguments: self.vm_arguments.clone(),
            program_arguments: self.program_arguments.clone(),
            result_port,
            testng_runner_path: self.testng_runner_path.clone(),
            testng_test_names: self.testng_test_names.clone(),
        }
    }
}

/// Combines a plan with JDT LS's runner arguments.
///
/// TestNG runs through the packaged runner, which the plugin package declares
/// as the `testRunner` resource; JUnit uses the main class JDT LS resolved.
pub fn debug_launch_target(
    file_path: &str,
    file_name: &str,
    plan: JavaTestLaunchPlan,
    launch: ResolvedJavaTestLaunchArguments,
    testng_runner_path: Option<&str>,
) -> Result<JavaTestDebugLaunchTarget, CoreError> {
    let (main_class, runner_path) = match plan.framework {
        JavaTestFramework::Testng => {
            let Some(runner) = testng_runner_path else {
                return Err(unavailable(
                    "The packaged Java TestNG runner is unavailable. Reinstall the Java Support plugin.",
                ));
            };
            if plan.testng_test_names.is_empty() {
                return Err(unavailable(&format!(
                    "No TestNG test method was found in {file_name}."
                )));
            }
            (
                TESTNG_RUNNER_MAIN_CLASS.to_string(),
                Some(runner.to_string()),
            )
        }
        JavaTestFramework::Junit => {
            let Some(main_class) = launch.main_class.clone() else {
                return Err(unavailable(
                    "The Java language service returned no JUnit runner main class.",
                ));
            };
            (main_class, None)
        }
    };
    Ok(JavaTestDebugLaunchTarget {
        file_path: file_path.to_string(),
        name: plan.name,
        framework: plan.framework,
        working_directory: launch.working_directory,
        main_class,
        project_name: launch.project_name,
        class_paths: launch.class_paths,
        module_paths: launch.module_paths,
        vm_arguments: launch.vm_arguments,
        program_arguments: launch.program_arguments,
        testng_runner_path: runner_path,
        testng_test_names: plan.testng_test_names,
    })
}

fn unavailable(message: &str) -> CoreError {
    CoreError::new(ErrorCode::NotSupported, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, full_name: &str, level: i64, children: Vec<JavaTestItem>) -> JavaTestItem {
        JavaTestItem {
            id: id.to_string(),
            label: full_name
                .rsplit(['.', '#'])
                .next()
                .unwrap_or(full_name)
                .to_string(),
            full_name: full_name.to_string(),
            project_name: "service".to_string(),
            kind: 1,
            level,
            jdt_handler: Some(format!("=service/{id}")),
            sort_text: None,
            range: None,
            children,
        }
    }

    fn user_service_test() -> Vec<JavaTestItem> {
        vec![item(
            "class",
            "example.UserServiceTest",
            CLASS_LEVEL,
            vec![
                item(
                    "b",
                    "example.UserServiceTest#logsOut()",
                    METHOD_LEVEL,
                    vec![],
                ),
                item(
                    "a",
                    "example.UserServiceTest#logsIn()",
                    METHOD_LEVEL,
                    vec![],
                ),
            ],
        )]
    }

    #[test]
    fn extension_json_uses_test_kind_and_level_keys() {
        let item: JavaTestItem = serde_json::from_value(json!({
            "id": "1", "label": "UserServiceTest", "fullName": "example.UserServiceTest",
            "projectName": "service", "testKind": 1, "testLevel": 5
        }))
        .unwrap();
        assert_eq!((item.kind, item.level), (1, CLASS_LEVEL));
        assert!(item.children.is_empty());
    }

    #[test]
    fn items_match_every_identity_the_host_holds() {
        let item = &user_service_test()[0];
        for identity in [
            "class",
            "UserServiceTest",
            "example.UserServiceTest",
            "=service/class",
        ] {
            assert!(item.matches(identity), "{identity}");
        }
        assert!(!item.matches("UserService"));
    }

    #[test]
    fn tests_tree_rows_are_sorted_nested_and_carry_report_names() {
        let rows = project_language_test_items(&user_service_test(), "/w/UserServiceTest.java", 1);
        let labels = rows
            .iter()
            .map(|row| (row.label.as_str(), row.depth))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![("UserServiceTest", 1), ("logsIn()", 2), ("logsOut()", 2)]
        );
        assert_eq!(
            rows[0].report_identity,
            Some(ReportIdentity {
                suite: "example.UserServiceTest".into(),
                name: None
            })
        );
        assert_eq!(
            rows[1].report_identity,
            Some(ReportIdentity {
                suite: "example.UserServiceTest".into(),
                name: Some("logsIn".into())
            })
        );
        assert_eq!(
            rows[1].test_identifier.as_deref(),
            Some("example.UserServiceTest#logsIn()")
        );
        assert!(rows
            .iter()
            .all(|row| row.kind == LanguageTestItemKind::TestCase && row.provider_id == "java"));
    }

    #[test]
    fn host_fields_survive_a_round_trip() {
        let row: LanguageTestItem = serde_json::from_value(json!({
            "id": "1", "providerId": "java", "label": "A", "kind": "file", "depth": 0,
            "projectPath": "service", "buildReference": {"tool": "maven"}
        }))
        .unwrap();
        let encoded = serde_json::to_value(&row).unwrap();
        assert_eq!(encoded["projectPath"], "service");
        assert_eq!(encoded["buildReference"]["tool"], "maven");
    }

    #[test]
    fn build_selectors_cover_the_file_or_exactly_one_test() {
        let items = user_service_test();
        assert_eq!(
            build_test_selectors(&items, None).unwrap(),
            vec!["example.UserServiceTest"]
        );
        assert_eq!(
            build_test_selectors(&items, Some("a")).unwrap(),
            vec!["example.UserServiceTest#logsIn()"]
        );
        assert!(build_test_selectors(&items, Some("missing")).is_err());
        assert!(build_test_selectors(&[], None).is_err());
    }

    #[test]
    fn a_junit_method_launch_uses_jdt_handles() {
        let plan =
            plan_debug_launch(&user_service_test(), "UserServiceTest.java", Some("a")).unwrap();
        assert_eq!(plan.framework, JavaTestFramework::Junit);
        assert_eq!(plan.name, "logsIn()");
        assert_eq!(plan.launch_test_names, vec!["=service/a"]);
        assert_eq!(
            plan.launch_arguments_request(),
            r#"{"projectName":"service","testKind":1,"testLevel":6,"testNames":["=service/a"]}"#
        );
    }

    #[test]
    fn a_file_launch_selects_its_classes_by_full_name() {
        let plan = plan_debug_launch(&user_service_test(), "UserServiceTest.java", None).unwrap();
        assert_eq!(plan.level, CLASS_LEVEL);
        assert_eq!(plan.launch_test_names, vec!["example.UserServiceTest"]);
        assert_eq!(plan.name, "UserServiceTest");
    }

    #[test]
    fn mixed_projects_or_frameworks_are_rejected() {
        let mut items = user_service_test();
        let mut other = items[0].clone();
        other.id = "other".into();
        other.project_name = "web".into();
        items.push(other);
        let error = plan_debug_launch(&items, "Mixed.java", None).unwrap_err();
        assert_eq!(
            error.message,
            "Debug one Java test framework and project at a time."
        );
        let error = plan_debug_launch(&items, "Mixed.java", Some("missing")).unwrap_err();
        assert_eq!(error.message, "No Java test was found for missing.");
    }

    #[test]
    fn a_junit_method_without_a_handle_is_incomplete() {
        let mut items = user_service_test();
        items[0].children[1].jdt_handler = None;
        let error = plan_debug_launch(&items, "UserServiceTest.java", Some("a")).unwrap_err();
        assert_eq!(
            error.message,
            "The Java language service returned incomplete test identifiers."
        );
    }

    #[test]
    fn testng_runs_through_the_packaged_runner() {
        let mut items = user_service_test();
        items[0].kind = 2;
        for child in &mut items[0].children {
            child.kind = 2;
        }
        let plan = plan_debug_launch(&items, "UserServiceTest.java", None).unwrap();
        assert_eq!(
            plan.testng_test_names,
            vec![
                "example.UserServiceTest#logsOut()",
                "example.UserServiceTest#logsIn()"
            ]
        );
        let launch = ResolvedJavaTestLaunchArguments {
            working_directory: "/w".into(),
            ..Default::default()
        };
        assert!(
            debug_launch_target("/w/T.java", "T.java", plan.clone(), launch.clone(), None).is_err()
        );
        let target =
            debug_launch_target("/w/T.java", "T.java", plan, launch, Some("/p/runner.jar"))
                .unwrap();
        assert_eq!(target.main_class, TESTNG_RUNNER_MAIN_CLASS);
        assert_eq!(target.testng_runner_path.as_deref(), Some("/p/runner.jar"));
        let request = target.launch_request(43_128);
        assert_eq!(request.result_port, 43_128);
    }

    #[test]
    fn launch_arguments_require_a_working_directory_and_surface_errors() {
        assert!(parse_launch_arguments(&json!([])).is_err());
        let error = parse_launch_arguments(&json!({"errorMessage": "no project"})).unwrap_err();
        assert_eq!(error.message, "no project");
        assert!(parse_launch_arguments(&json!({"body": {"workingDirectory": ""}})).is_err());
        let parsed = parse_launch_arguments(&json!({"body": {
            "workingDirectory": "/w", "mainClass": "Runner", "projectName": "",
            "classpath": ["/a", "", 3], "vmArguments": ["-ea"]
        }}))
        .unwrap();
        assert_eq!(parsed.main_class.as_deref(), Some("Runner"));
        assert_eq!(parsed.project_name, None);
        assert_eq!(parsed.class_paths, vec!["/a"]);
        assert_eq!(parsed.vm_arguments, vec!["-ea"]);
    }

    #[test]
    fn junit_needs_the_runner_main_class_from_jdt() {
        let plan = plan_debug_launch(&user_service_test(), "UserServiceTest.java", None).unwrap();
        let launch = ResolvedJavaTestLaunchArguments {
            working_directory: "/w".into(),
            ..Default::default()
        };
        let error = debug_launch_target("/w/T.java", "T.java", plan, launch, None).unwrap_err();
        assert_eq!(
            error.message,
            "The Java language service returned no JUnit runner main class."
        );
    }
}
