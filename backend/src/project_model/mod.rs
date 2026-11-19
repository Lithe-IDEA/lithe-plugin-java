//! The Java project model of a workspace folder, and what the plugin adds to
//! JDT LS for the folders JDT imports as a plain ("invisible") project.
//!
//! JDT LS owns import: Maven and Gradle build files win, then Eclipse
//! `.project` files, and only a folder with neither becomes an invisible
//! project, whose classpath comes from `java.project.referencedLibraries`
//! and `java.project.sourcePaths` alone. For those folders this module reads
//! the layouts other tools leave behind (IntelliJ IDEA modules, legacy web
//! roots with `WEB-INF/lib`, exploded EARs) and adds what they declare to the
//! two settings **in memory**: the user's own `.vscode/settings.json` values
//! stay first and in charge, and no project file is written unless the user
//! saves through the settings page. Eclipse WTP containers are resolved inside
//! JDT LS by the plugin's extension bundle (`jdtls-extension/project-model`),
//! which receives the servlet container under `java.lithe.projectModel`.
//!
//! Everything here is bounded: one walk ([`scan`]), fixed descriptor paths,
//! 2 MiB per descriptor, and item limits in each reader.

pub mod eclipse;
mod idea;
pub(crate) use idea::IdeaModule;
mod scan;
pub mod servlet;
mod web;
mod xml;

use serde_json::{json, Value};
use servlet::{ServletEnvironment, ServletResolution};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Plugin setting that turns detection (and the bundle's containers) off.
pub const DETECTION_SETTING: &str = "projectModelDetection";
/// Plugin setting naming the servlet container folder; empty is automatic.
pub const SERVLET_HOME_SETTING: &str = "servletContainerHome";
/// The upstream default of `java.project.referencedLibraries`. Setting the key
/// replaces the default, so an overlay without an explicit value keeps it.
pub const UPSTREAM_DEFAULT_LIBRARIES: &str = "lib/**";
pub const CLASS_FOLDERS_KEY: &str = "java.lithe.classFolders";
/// Largest descriptor file read.
pub const MAX_DESCRIPTOR_BYTES: u64 = 2 * 1024 * 1024;

/// The plugin settings that steer detection, from a launch's `settings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOptions {
    pub detection: bool,
    pub servlet_home: Option<PathBuf>,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            detection: true,
            servlet_home: None,
        }
    }
}

/// Reads [`DETECTION_SETTING`] and [`SERVLET_HOME_SETTING`]; absent values
/// are the manifest defaults.
pub fn parse_options(settings: &Value) -> Result<ModelOptions, &'static str> {
    let detection = match settings.get(DETECTION_SETTING) {
        None | Some(Value::Null) => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("The project model detection setting is not a boolean."),
    };
    let servlet_home = match settings.get(SERVLET_HOME_SETTING) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) if Path::new(text).is_absolute() => Some(PathBuf::from(text)),
        Some(_) => return Err("The servlet container folder setting is not a folder."),
    };
    Ok(ModelOptions {
        detection,
        servlet_home,
    })
}

/// Which JDT importer takes the folder, and which layout fed the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Maven,
    Gradle,
    Eclipse,
    Idea,
    LegacyWeb,
    Plain,
}

impl ModelKind {
    /// Whether a JDT importer other than the invisible project owns the
    /// classpath; overlays of `java.project.*` then have no effect.
    pub fn managed(self) -> bool {
        matches!(self, Self::Maven | Self::Gradle | Self::Eclipse)
    }
}

/// Where a detected library comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    WebLibraries,
    EarLibraries,
    ManifestClassPath,
    IdeaModuleLibrary,
    IdeaProjectLibrary,
    ServletContainer,
}

/// One `referencedLibraries` include the model adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// Workspace-relative glob or file (slash separated), or an absolute path.
    pub pattern: String,
    pub origin: Origin,
    /// Module, library or folder name shown with the entry.
    pub label: String,
    /// Number of JAR files the entry stands for (displayed only).
    pub count: usize,
}

/// Why a declared library could not be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedKind {
    /// A declared file does not exist.
    MissingFile,
    /// A project library a module uses is not defined.
    MissingLibrary,
    /// An IDEA application or global library, kept outside the project.
    IdeLibrary,
    /// A path variable whose value only the other IDE knows.
    UnknownMacro,
    /// A source folder outside the workspace folder.
    OutsideWorkspace,
    /// An Eclipse user library, defined in an Eclipse workspace.
    UserLibrary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub kind: UnresolvedKind,
    pub name: String,
}

/// The detected model of one workspace folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectModel {
    pub kind: ModelKind,
    pub enabled: bool,
    /// Workspace-relative descriptor that identifies the model, if any.
    pub descriptor: Option<String>,
    pub idea_modules: usize,
    pub(crate) idea_module_details: Vec<IdeaModule>,
    pub eclipse_projects: usize,
    /// Libraries for the invisible project; empty for managed kinds.
    pub libraries: Vec<Detected>,
    /// IDEA source folders, production before test.
    pub source_paths: Vec<String>,
    pub unresolved: Vec<Unresolved>,
    /// The project needs servlet container libraries.
    pub needs_servlet: bool,
    /// Resolved only when [`Self::needs_servlet`].
    pub servlet: ServletResolution,
    /// A scan or reader limit cut detection short.
    pub truncated: bool,
}

/// What detection may read beyond the workspace: the user's home for IDEA
/// macros and the servlet container search.
pub struct Environment {
    pub servlet: ServletEnvironment,
    pub user_home: Option<PathBuf>,
}

impl Environment {
    /// The process environment of this platform.
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "macos",
            "windows" => "windows",
            _ => "linux",
        };
        let user_home = std::env::var_os(if os == "windows" {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
        Self {
            servlet: ServletEnvironment::current(os),
            user_home,
        }
    }
}

/// Detects the model of the folder `root`.
pub fn detect(root: &Path, options: &ModelOptions, environment: &Environment) -> ProjectModel {
    let found = scan::scan(root);
    let mut model = ProjectModel {
        kind: ModelKind::Plain,
        enabled: options.detection,
        descriptor: None,
        idea_modules: 0,
        idea_module_details: Vec::new(),
        eclipse_projects: found.eclipse_projects.len(),
        libraries: Vec::new(),
        source_paths: Vec::new(),
        unresolved: Vec::new(),
        needs_servlet: false,
        servlet: ServletResolution::default(),
        truncated: found.truncated,
    };
    let mut hints = Vec::new();
    if let Some(build) = found.build_files.first() {
        let maven = found
            .build_files
            .iter()
            .any(|file| file.ends_with("pom.xml"));
        model.kind = if maven {
            ModelKind::Maven
        } else {
            ModelKind::Gradle
        };
        let descriptor = found
            .build_files
            .iter()
            .find(|file| !maven || file.ends_with("pom.xml"))
            .unwrap_or(build);
        model.descriptor = Some(relative_or_absolute(root, descriptor));
        return model;
    }
    if let Some(project) = found.eclipse_projects.first() {
        model.kind = ModelKind::Eclipse;
        model.descriptor = Some(relative_or_absolute(root, &project.join(".project")));
        if options.detection {
            let facts = eclipse::read(&found.eclipse_projects);
            model.needs_servlet = facts.needs_servlet;
            hints.extend(facts.hints);
            model
                .unresolved
                .extend(facts.user_libraries.into_iter().map(|name| Unresolved {
                    kind: UnresolvedKind::UserLibrary,
                    name,
                }));
        }
    } else {
        let macros = idea::Macros {
            project_dir: root.to_path_buf(),
            user_home: environment.user_home.clone(),
            maven_repository: environment
                .user_home
                .as_ref()
                .map(|home| home.join(".m2/repository")),
        };
        let idea = idea::read(root, &found.ipr_files, &macros);
        let mut web_roots = found.web_roots.clone();
        if let Some(idea) = &idea {
            model.kind = ModelKind::Idea;
            model.descriptor = Some(idea.descriptor.clone());
            model.idea_modules = idea.modules;
            for web_root in &idea.web_roots {
                if !web_roots.contains(web_root) && web_root.join("WEB-INF").is_dir() {
                    web_roots.push(web_root.clone());
                }
            }
        } else if let Some(web_root) = web_roots.first() {
            model.kind = ModelKind::LegacyWeb;
            model.descriptor = Some(relative_or_absolute(
                root,
                &web_root.join("WEB-INF/web.xml"),
            ));
        } else if let Some(ear_root) = found.ear_roots.first() {
            model.kind = ModelKind::LegacyWeb;
            model.descriptor = Some(relative_or_absolute(
                root,
                &ear_root.join("META-INF/application.xml"),
            ));
        }
        if !options.detection {
            return model;
        }
        if let Some(idea) = idea {
            model.source_paths = idea
                .source_paths
                .into_iter()
                .chain(idea.test_source_paths)
                .collect();
            model.libraries = idea.libraries;
            model.idea_module_details = idea.module_details;
            model.unresolved = idea.unresolved;
            model.needs_servlet = idea.needs_servlet;
            model.truncated |= idea.truncated;
            hints.extend(idea.hints);
        }
        let web = web::read(root, &web_roots, &found.ear_roots);
        if let Some(namespace) = web.namespace {
            hints.push(servlet::ServletHint::Namespace(namespace));
        }
        for library in web.libraries {
            if !model
                .libraries
                .iter()
                .any(|known| known.pattern == library.pattern)
            {
                model.libraries.push(library);
            }
        }
        model.needs_servlet |= !web_roots.is_empty();
    }
    if model.needs_servlet {
        model.servlet = servlet::resolve(
            options.servlet_home.as_deref(),
            &hints,
            &environment.servlet,
        );
    }
    model
}

/// The `java.project.*` values of a `.vscode/settings.json` document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Explicit {
    /// `referencedLibraries`: an include array or the rich object form.
    pub libraries: Option<Value>,
    pub source_paths: Option<Value>,
    pub output_path: Option<Value>,
    pub class_folders: Option<Value>,
    pub source_roles: BTreeMap<&'static str, Vec<String>>,
}

pub(crate) const SOURCE_ROLE_KEYS: [(&str, &str); 4] = [
    ("test", "java.lithe.testSourceFolders"),
    ("resource", "java.lithe.resourceFolders"),
    ("test-resource", "java.lithe.testResourceFolders"),
    ("excluded", "java.lithe.excludedSourceFolders"),
];

impl Explicit {
    pub fn from_document(document: &Value) -> Self {
        Self {
            libraries: document.get("java.project.referencedLibraries").cloned(),
            source_paths: document.get("java.project.sourcePaths").cloned(),
            output_path: document.get("java.project.outputPath").cloned(),
            class_folders: document.get(CLASS_FOLDERS_KEY).cloned(),
            source_roles: SOURCE_ROLE_KEYS
                .into_iter()
                .filter_map(|(kind, key)| {
                    let paths = strings(document.get(key));
                    (!paths.is_empty()).then_some((kind, paths))
                })
                .collect(),
        }
    }

    /// The explicit include patterns, from either form.
    pub fn include(&self) -> Vec<String> {
        let list = match &self.libraries {
            Some(Value::Object(object)) => object.get("include"),
            other => other.as_ref(),
        };
        strings(list)
    }
}

/// Every include the model adds: detected libraries, then the servlet
/// container's libraries. Empty for managed kinds and when disabled.
pub fn detected_include(model: &ProjectModel) -> Vec<String> {
    if !model.enabled || model.kind.managed() {
        return Vec::new();
    }
    let mut include: Vec<String> = model
        .libraries
        .iter()
        .map(|item| item.pattern.clone())
        .collect();
    if let Some(runtime) = &model.servlet.runtime {
        include.extend(
            runtime
                .libraries
                .iter()
                .map(|path| path.to_string_lossy().to_string()),
        );
    }
    include
}

/// The effective `referencedLibraries`: the explicit value (or the upstream
/// default) with the detected includes appended, keeping its excludes and
/// source attachments. None when neither exists: JDT LS rebuilds its
/// preferences from every push, so an absent key is the upstream default.
pub fn effective_libraries(model: &ProjectModel, explicit: &Explicit) -> Option<Value> {
    let detected = detected_include(model);
    if detected.is_empty() {
        return explicit.libraries.clone();
    }
    let mut include = match &explicit.libraries {
        Some(_) => explicit.include(),
        None => vec![UPSTREAM_DEFAULT_LIBRARIES.to_string()],
    };
    for pattern in detected {
        if !include.contains(&pattern) {
            include.push(pattern);
        }
    }
    Some(match &explicit.libraries {
        Some(Value::Object(object)) => {
            let mut object = object.clone();
            object.insert("include".into(), json!(include));
            Value::Object(object)
        }
        _ => json!(include),
    })
}

/// The effective `sourcePaths`: explicit, else the IDEA source folders.
pub fn effective_source_paths(model: &ProjectModel, explicit: &Explicit) -> Option<Value> {
    if explicit.source_paths.is_some() {
        return explicit.source_paths.clone();
    }
    (model.enabled && model.kind == ModelKind::Idea && !model.source_paths.is_empty())
        .then(|| json!(model.source_paths))
}

/// The `java` settings sections the plugin overlays for `model`:
/// `{project: {...}, lithe: {projectModel: {...}}}`, each only with keys
/// that differ from what JDT LS and the bundle assume when a key is absent.
pub fn overlay(root: &Path, model: &ProjectModel, explicit: &Explicit) -> Value {
    let mut project = json!({});
    if let Some(libraries) = effective_libraries(model, explicit) {
        project["referencedLibraries"] = libraries;
    }
    if let Some(paths) = effective_source_paths(model, explicit) {
        project["sourcePaths"] = paths;
    }
    if let Some(output) = &explicit.output_path {
        project["outputPath"] = output.clone();
    }
    let runtime = model.servlet.runtime.as_ref();
    let mut seen = std::collections::BTreeSet::new();
    let class_folders: Vec<_> = if model.kind.managed() {
        Vec::new()
    } else {
        strings(explicit.class_folders.as_ref())
            .into_iter()
            .chain(model.idea_module_details.iter().flat_map(|module| {
                module
                    .class_folders
                    .iter()
                    .map(|folder| folder.pattern.clone())
            }))
            .filter(|path| !path.trim().is_empty() && path.len() <= 4096)
            .map(|path| root.join(path))
            .filter(|path| seen.insert(path.clone()))
            .take(1024)
            .collect()
    };
    let roles = if matches!(model.kind, ModelKind::Plain | ModelKind::LegacyWeb) {
        let mut roles = BTreeMap::new();
        for (&kind, paths) in &explicit.source_roles {
            let paths: Vec<_> = paths
                .iter()
                .take(256)
                .filter(|path| crate::library_settings::valid_path(path))
                .filter_map(|path| dunce::canonicalize(root.join(path)).ok())
                .filter(|path| path.is_dir() && path.starts_with(root))
                .collect();
            if !paths.is_empty() {
                roles.insert(kind, paths);
            }
        }
        roles
    } else {
        BTreeMap::new()
    };
    let workspace_module = matches!(model.kind, ModelKind::Plain | ModelKind::LegacyWeb);
    let workspace_source_paths = workspace_module
        .then_some(explicit.source_paths.as_ref())
        .flatten()
        .filter(|paths| {
            paths
                .as_array()
                .is_some_and(|paths| !paths.is_empty() || roles.contains_key("excluded"))
        });
    let manages_workspace_paths =
        workspace_module && (workspace_source_paths.is_some() || explicit.output_path.is_some());
    if model.enabled
        && runtime.is_none()
        && class_folders.is_empty()
        && roles.is_empty()
        && !manages_workspace_paths
    {
        return json!({"project": project});
    }
    json!({
        "project": project,
        "lithe": {"projectModel": {
            "enabled": model.enabled,
            "classFolderWorkspace": root,
            "classFolders": class_folders,
            "sourceRoles": roles,
            "workspaceSourcePaths": workspace_source_paths,
            "workspaceOutputPath": workspace_module.then(|| explicit.output_path.as_ref().and_then(Value::as_str).unwrap_or("")),
            "serverRuntimeHome": runtime.map(|runtime| runtime.home.to_string_lossy().to_string()).unwrap_or_default(),
            "serverRuntimeLibraries": runtime.map(|runtime| runtime.libraries.iter().map(|path| path.to_string_lossy().to_string()).collect::<Vec<_>>()).unwrap_or_default(),
            "serverRuntimeName": runtime.map(|runtime| runtime.name.clone()).unwrap_or_default(),
        }},
    })
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The bytes of a regular file of at most [`MAX_DESCRIPTOR_BYTES`].
pub(crate) fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let info = fs::metadata(path).ok()?;
    if !info.is_file() || info.len() > MAX_DESCRIPTOR_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_DESCRIPTOR_BYTES).then_some(bytes)
}

/// `path` relative to `root` with `/` separators, or None outside it.
pub(crate) fn relative_inside(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    Some(if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    })
}

/// `path` relative to `root` when inside it, else the absolute path.
pub(crate) fn relative_or_absolute(root: &Path, path: &Path) -> String {
    relative_inside(root, path).unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A temporary folder tree removed on drop; `root` is canonical.
    pub struct Tree {
        pub root: PathBuf,
    }

    impl Tree {
        pub fn new(files: &[(&str, &str)]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let base = std::env::temp_dir().join(format!(
                "lithe-project-model-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(base.join("workspace")).unwrap();
            let tree = Self {
                root: dunce::canonicalize(base.join("workspace")).unwrap(),
            };
            for (path, content) in files {
                tree.write(path, content);
            }
            tree
        }

        pub fn write(&self, path: &str, content: &str) {
            let file = self.root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            if let Some(base) = self.root.parent() {
                if let Err(error) = fs::remove_dir_all(base) {
                    if !std::thread::panicking() {
                        panic!("temporary tree cleanup: {error}");
                    }
                }
            }
        }
    }

    fn environment(tree: &Tree) -> Environment {
        Environment {
            servlet: ServletEnvironment {
                variables: vec![],
                search: vec![(tree.root.join("servers"), None)],
            },
            user_home: Some(tree.root.join("home")),
        }
    }

    fn tomcat(tree: &Tree) -> PathBuf {
        tree.write("servers/apache-tomcat-9.0.98/lib/servlet-api.jar", "");
        tree.write("servers/apache-tomcat-9.0.98/lib/jsp-api.jar", "");
        // Joined by component: the backend reports native separators.
        tree.root.join("servers").join("apache-tomcat-9.0.98")
    }

    #[test]
    fn a_legacy_web_folder_keeps_the_upstream_default_and_adds_web_libraries_and_the_servlet_container(
    ) {
        let tree = Tree::new(&[
            ("project/WebContent/WEB-INF/lib/a.jar", ""),
            ("project/src/demo/A.java", ""),
        ]);
        let home = tomcat(&tree);
        let model = detect(&tree.root, &ModelOptions::default(), &environment(&tree));
        assert_eq!(model.kind, ModelKind::LegacyWeb);
        assert_eq!(
            model.descriptor.as_deref(),
            Some("project/WebContent/WEB-INF/web.xml")
        );
        let value = overlay(&tree.root, &model, &Explicit::default());
        assert_eq!(
            value["project"]["referencedLibraries"],
            json!([
                "lib/**",
                "project/WebContent/WEB-INF/lib/*.jar",
                home.join("lib").join("jsp-api.jar").to_str().unwrap(),
                home.join("lib").join("servlet-api.jar").to_str().unwrap(),
            ])
        );
        assert!(value["project"].get("sourcePaths").is_none());
        assert_eq!(
            value["lithe"]["projectModel"]["serverRuntimeName"],
            "Apache Tomcat 9.0.98"
        );
        assert_eq!(
            value["lithe"]["projectModel"]["serverRuntimeHome"],
            home.to_str().unwrap()
        );
    }

    #[test]
    fn excluding_the_only_source_root_overrides_jdt_with_an_empty_source_list() {
        let tree = Tree::new(&[("src/Main.java", "")]);
        let root = dunce::canonicalize(&tree.root).unwrap();
        let model = detect(&root, &ModelOptions::default(), &environment(&tree));
        let explicit = Explicit::from_document(&json!({
            "java.project.sourcePaths": [],
            "java.lithe.excludedSourceFolders": ["src"],
        }));
        let value = overlay(&root, &model, &explicit);
        assert_eq!(
            value["lithe"]["projectModel"]["workspaceSourcePaths"],
            json!([])
        );
        let inherited = Explicit::from_document(&json!({"java.project.sourcePaths": []}));
        assert!(overlay(&root, &model, &inherited).get("lithe").is_none());
    }

    #[test]
    fn explicit_values_stay_first_and_keep_excludes_and_sources() {
        let tree = Tree::new(&[("WebRoot/WEB-INF/lib/a.jar", "")]);
        let model = detect(&tree.root, &ModelOptions::default(), &environment(&tree));
        let explicit = Explicit::from_document(&json!({
            "java.project.referencedLibraries": {"include": ["vendor/x.jar"], "exclude": ["WebRoot/WEB-INF/lib/bad.jar"], "sources": {"vendor/x.jar": "vendor/x-src.jar"}},
            "java.project.sourcePaths": ["src/java"],
            "java.project.outputPath": "out",
        }));
        let value = overlay(&tree.root, &model, &explicit);
        assert_eq!(
            value["project"]["referencedLibraries"]["include"],
            json!(["vendor/x.jar", "WebRoot/WEB-INF/lib/*.jar"])
        );
        assert_eq!(
            value["project"]["referencedLibraries"]["exclude"],
            json!(["WebRoot/WEB-INF/lib/bad.jar"])
        );
        assert_eq!(
            value["project"]["referencedLibraries"]["sources"]["vendor/x.jar"],
            "vendor/x-src.jar"
        );
        assert_eq!(value["project"]["sourcePaths"], json!(["src/java"]));
        assert_eq!(value["project"]["outputPath"], "out");
        let bundle = &value["lithe"]["projectModel"];
        assert_eq!(bundle["workspaceSourcePaths"], json!(["src/java"]));
        assert_eq!(bundle["workspaceOutputPath"], "out");
        assert_eq!(bundle["serverRuntimeLibraries"], json!([]));
    }

    #[test]
    fn idea_projects_set_source_paths_and_build_files_win_over_every_layout() {
        let tree = Tree::new(&[
            (
                ".idea/modules.xml",
                r#"<project><component name="ProjectModuleManager"><modules><module filepath="$PROJECT_DIR$/a.iml"/></modules></component></project>"#,
            ),
            (
                "a.iml",
                r#"<module><component name="NewModuleRootManager"><content url="file://$MODULE_DIR$"><sourceFolder url="file://$MODULE_DIR$/java" isTestSource="false"/><sourceFolder url="file://$MODULE_DIR$/tests" isTestSource="true"/></content></component></module>"#,
            ),
        ]);
        let model = detect(&tree.root, &ModelOptions::default(), &environment(&tree));
        assert_eq!(model.kind, ModelKind::Idea);
        let value = overlay(&tree.root, &model, &Explicit::default());
        assert_eq!(value["project"]["sourcePaths"], json!(["java", "tests"]));
        assert!(value["project"].get("referencedLibraries").is_none());
        assert!(value.get("lithe").is_none());
        tree.write("module/build.gradle", "");
        let model = detect(&tree.root, &ModelOptions::default(), &environment(&tree));
        assert_eq!(
            (model.kind, model.descriptor.as_deref()),
            (ModelKind::Gradle, Some("module/build.gradle"))
        );
        assert_eq!(
            overlay(&tree.root, &model, &Explicit::default())["project"],
            json!({})
        );
    }

    #[test]
    fn eclipse_projects_only_configure_the_servlet_container_for_the_bundle() {
        let tree = Tree::new(&[
            ("app/.project", "<projectDescription/>"),
            (
                "app/.classpath",
                r#"<classpath><classpathentry kind="con" path="org.eclipse.jst.server.core.container/x/Apache Tomcat v9.0"/><classpathentry kind="con" path="org.eclipse.jdt.USER_LIBRARY/Libs"/></classpath>"#,
            ),
            ("app/WebContent/WEB-INF/lib/a.jar", ""),
        ]);
        let home = tomcat(&tree);
        let model = detect(&tree.root, &ModelOptions::default(), &environment(&tree));
        assert_eq!(
            (model.kind, model.descriptor.as_deref()),
            (ModelKind::Eclipse, Some("app/.project"))
        );
        assert_eq!(
            model.unresolved,
            vec![Unresolved {
                kind: UnresolvedKind::UserLibrary,
                name: "Libs".into()
            }]
        );
        let value = overlay(&tree.root, &model, &Explicit::default());
        assert_eq!(value["project"], json!({}));
        assert_eq!(
            value["lithe"]["projectModel"]["serverRuntimeHome"],
            home.to_str().unwrap()
        );
    }

    #[test]
    fn disabling_detection_changes_nothing_but_tells_the_bundle() {
        let tree = Tree::new(&[("WebContent/WEB-INF/lib/a.jar", "")]);
        tomcat(&tree);
        let options = ModelOptions {
            detection: false,
            servlet_home: None,
        };
        let model = detect(&tree.root, &options, &environment(&tree));
        assert_eq!(model.kind, ModelKind::LegacyWeb);
        let value = overlay(&tree.root, &model, &Explicit::default());
        assert_eq!(value["project"], json!({}));
        assert_eq!(value["lithe"]["projectModel"]["enabled"], false);
    }

    #[test]
    fn options_are_parsed_strictly() {
        assert_eq!(parse_options(&json!({})), Ok(ModelOptions::default()));
        let folder = if cfg!(windows) {
            r"C:\tomcat"
        } else {
            "/opt/tomcat"
        };
        assert_eq!(
            parse_options(&json!({DETECTION_SETTING: false, SERVLET_HOME_SETTING: folder})),
            Ok(ModelOptions {
                detection: false,
                servlet_home: Some(PathBuf::from(folder))
            })
        );
        assert_eq!(
            parse_options(&json!({SERVLET_HOME_SETTING: ""})),
            Ok(ModelOptions::default())
        );
        assert!(parse_options(&json!({SERVLET_HOME_SETTING: "relative"})).is_err());
        assert!(parse_options(&json!({DETECTION_SETTING: "yes"})).is_err());
    }
}
