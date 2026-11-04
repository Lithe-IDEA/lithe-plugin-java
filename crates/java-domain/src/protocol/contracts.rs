//! Serializable response models of the Java commands.
//!
//! Copied from `rust/lithe-core/src/protocol/contracts.rs` in the Lithe
//! repository. Only the Java, Spring and MyBatis models move here; the field
//! names and JSON shapes stay identical so the plugin backend can answer the
//! same requests Rust Core answered before the split.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Java class containing a runnable main method.
pub struct JavaMainClassResponse {
    pub path: String,
    pub qualified_name: String,
    pub simple_name: String,
    pub is_spring_boot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Source-set classification attached to one discovered Java run entry.
pub enum JavaSourceSetResponse {
    /// Production source compiled into the main project output.
    Main,
    /// Test source compiled into the test project output.
    Test,
    /// Java source outside the conventional Maven main and test layouts.
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// UI-facing Java or Spring Boot run entry.
pub struct JavaRunConfigurationResponse {
    pub id: String,
    pub name: String,
    /// UI category such as `javaMain`, `springBoot`, or `mavenModule`.
    pub kind: String,
    pub module_path: Option<String>,
    pub main_class: Option<String>,
    /// Workspace-relative source that produced this exact run entry.
    pub source_path: String,
    /// Build output that must be present on the Java launch classpath.
    pub source_set: JavaSourceSetResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Discovered Java main classes and their stable run entries.
pub struct JavaRunConfigurationsResponse {
    pub main_classes: Vec<JavaMainClassResponse>,
    pub configurations: Vec<JavaRunConfigurationResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Usage count rendered above one Java declaration.
pub struct JavaCodeVisionHintResponse {
    pub line: usize,
    pub utf16_column: usize,
    pub symbol: String,
    pub usage_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Code Vision hints in source order.
pub struct JavaCodeVisionResponse {
    pub hints: Vec<JavaCodeVisionHintResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Package-qualified Java class name.
pub struct JavaClassNameResponse {
    pub class_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Zero-based UTF-16 location of a Java declaration.
pub struct JavaSourceDefinitionResponse {
    pub line: usize,
    pub utf16_column: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Server port declared by Spring configuration, when one is present.
pub struct JavaServerPortResponse {
    pub port: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Foldable Java source region and the text span hidden by folding.
pub struct JavaFoldRegionResponse {
    /// Fold category such as imports, declaration, or comment.
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub hidden_start: usize,
    pub hidden_length: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Lightweight Java parameter-name inlay hint.
pub struct JavaInlayHintResponse {
    pub line: usize,
    pub utf16_column: usize,
    pub label: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// One non-overlapping Java token range with a portable editor-theme role.
pub struct JavaSyntaxHighlightResponse {
    /// Document-relative start measured in UTF-16 code units.
    pub utf16_start: usize,
    /// Token length measured in UTF-16 code units.
    pub utf16_length: usize,
    /// Semantic role defined by the shared editor syntax-theme contract.
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Lightweight structural features derived from one Java source document.
pub struct JavaStructureResponse {
    pub fold_regions: Vec<JavaFoldRegionResponse>,
    pub inlay_hints: Vec<JavaInlayHintResponse>,
    pub syntax_highlights: Vec<JavaSyntaxHighlightResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// One Java navigation gutter marker normalized from provider semantic data.
pub struct JavaNavigationMarkerResponse {
    pub line: usize,
    pub utf16_column: usize,
    pub implementation_count: usize,
    pub direction: String,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// One Spring configuration property and its optional Java declaration.
pub struct SpringPropertyResponse {
    pub name: String,
    pub type_name: Option<String>,
    pub description: Option<String>,
    pub default_value: Option<String>,
    pub source_path: Option<String>,
    pub source_line: Option<usize>,
    pub source_column: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// One key/value occurrence from a Spring application configuration document.
pub struct SpringConfigurationValueResponse {
    pub key: String,
    pub value: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub profile: Option<String>,
    pub overrides_base_value: bool,
    pub target_path: Option<String>,
    pub target_line: Option<usize>,
    pub target_column: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Java `@Value` reference to a Spring configuration property.
pub struct SpringPropertyReferenceResponse {
    pub key: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Spring configuration problem projected onto one source location.
pub struct SpringDiagnosticResponse {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Component or `@Bean` declaration available for dependency injection.
pub struct SpringBeanResponse {
    pub id: String,
    pub name: String,
    pub type_name: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Injection point and the bean declarations that satisfy it.
pub struct SpringInjectionResponse {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub type_name: String,
    pub qualifier: Option<String>,
    pub bean_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// HTTP endpoint declared by a Spring MVC controller method.
pub struct SpringEndpointResponse {
    pub id: String,
    pub http_methods: Vec<String>,
    pub route: String,
    pub controller: String,
    pub method: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Complete deterministic Spring semantic index for one workspace snapshot.
pub struct SpringIndexResponse {
    pub properties: Vec<SpringPropertyResponse>,
    pub values: Vec<SpringConfigurationValueResponse>,
    pub property_references: Vec<SpringPropertyReferenceResponse>,
    pub diagnostics: Vec<SpringDiagnosticResponse>,
    pub beans: Vec<SpringBeanResponse>,
    pub injections: Vec<SpringInjectionResponse>,
    pub endpoints: Vec<SpringEndpointResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// One MyBatis statement that has both a Java mapper method and an XML `id`.
pub struct MybatisStatementResponse {
    /// Stable identity: namespace, statement id, XML path, and XML line.
    pub id: String,
    /// Fully qualified mapper type from the XML `namespace`.
    pub namespace: String,
    /// XML statement `id`, which matches the Java method name.
    pub statement_id: String,
    /// MyBatis statement kind: `select`, `insert`, `update`, or `delete`.
    pub kind: String,
    /// Workspace-relative Java mapper path.
    pub java_path: String,
    /// One-based line of the Java method name.
    pub java_line: usize,
    /// One-based UTF-16 column of the Java method name.
    pub java_column: usize,
    /// One-based line of the Java method signature terminator.
    pub java_end_line: usize,
    /// Exclusive one-based UTF-16 column after the Java method name.
    pub java_end_column: usize,
    /// Workspace-relative mapper XML path.
    pub xml_path: String,
    /// One-based line of the XML statement `id` value.
    pub xml_line: usize,
    /// One-based UTF-16 column of the XML statement `id` value.
    pub xml_column: usize,
    /// Exclusive one-based UTF-16 column after the XML statement `id` value.
    pub xml_end_column: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Complete deterministic MyBatis mapper/XML index for one workspace snapshot.
pub struct MybatisIndexResponse {
    /// Paired Java methods and XML statements, ordered by namespace and id.
    pub statements: Vec<MybatisStatementResponse>,
}
