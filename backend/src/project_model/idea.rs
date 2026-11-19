//! Read-only import of IntelliJ IDEA project models: `.idea/modules.xml` (or a
//! file-based `.ipr`), each module's `.iml` source folders and libraries, and
//! the project libraries in `.idea/libraries`. No IDEA file is ever written.
//! Libraries IDEA keeps outside the project (application and global
//! libraries) cannot be read and are reported as unresolved.

use super::servlet::ServletHint;
use super::{
    read_bounded, relative_inside, relative_or_absolute, xml, Detected, Origin, Unresolved,
    UnresolvedKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use xmltree::Element;

/// Most modules imported from one project.
pub const MAX_MODULES: usize = 512;
/// Most project library files read from `.idea/libraries`.
pub const MAX_LIBRARY_FILES: usize = 512;
/// Most library roots imported in total.
pub const MAX_LIBRARY_ROOTS: usize = 4096;

/// Folders the IDEA path macros stand for.
pub struct Macros {
    pub project_dir: PathBuf,
    pub user_home: Option<PathBuf>,
    pub maven_repository: Option<PathBuf>,
}

/// What an IDEA project contributes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IdeaFacts {
    /// Workspace-relative path of the descriptor that was read.
    pub descriptor: String,
    pub modules: usize,
    pub module_details: Vec<IdeaModule>,
    /// Workspace-relative production source folders, then test ones.
    pub source_paths: Vec<String>,
    pub test_source_paths: Vec<String>,
    pub content_roots: Vec<String>,
    pub resources: Vec<String>,
    pub test_resources: Vec<String>,
    pub excluded: Vec<String>,
    pub libraries: Vec<Detected>,
    pub class_folders: Vec<Detected>,
    pub module_dependencies: Vec<String>,
    pub output: Option<String>,
    pub test_output: Option<String>,
    pub inherits_output: bool,
    pub sdk: Option<String>,
    pub unresolved: Vec<Unresolved>,
    /// Web roots declared by web facets.
    pub web_roots: Vec<PathBuf>,
    /// A module is a web module or uses application server libraries.
    pub needs_servlet: bool,
    pub hints: Vec<ServletHint>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaModule {
    pub descriptor: String,
    pub name: String,
    pub sources: Vec<String>,
    pub test_sources: Vec<String>,
    pub content_roots: Vec<String>,
    pub resources: Vec<String>,
    pub test_resources: Vec<String>,
    pub excluded: Vec<String>,
    pub libraries: Vec<Detected>,
    pub class_folders: Vec<Detected>,
    pub module_dependencies: Vec<String>,
    pub unresolved: Vec<Unresolved>,
    pub output: Option<String>,
    pub test_output: Option<String>,
    pub inherits_output: bool,
    pub sdk: Option<String>,
}

/// Reads the IDEA project at `root`, or None when there is none.
pub fn read(root: &Path, ipr_files: &[PathBuf], macros: &Macros) -> Option<IdeaFacts> {
    let directory = root.join(".idea");
    let (descriptor, modules_document, project_libraries) = if let Some(document) =
        read_bounded(&directory.join("modules.xml")).and_then(|bytes| xml::parse(&bytes))
    {
        (
            ".idea/modules.xml".to_string(),
            document,
            directory_libraries(&directory.join("libraries")),
        )
    } else {
        let ipr = ipr_files.first()?;
        let document = read_bounded(ipr).and_then(|bytes| xml::parse(&bytes))?;
        let libraries = component(&document, "libraryTable")
            .map(named_libraries)
            .unwrap_or_default();
        (relative_or_absolute(root, ipr), document, libraries)
    };
    let mut facts = IdeaFacts {
        descriptor,
        ..IdeaFacts::default()
    };
    let misc = read_bounded(&directory.join("misc.xml")).and_then(|bytes| xml::parse(&bytes));
    let project_output = component(
        misc.as_ref().unwrap_or(&modules_document),
        "ProjectRootManager",
    )
    .and_then(|manager| xml::children(manager, "output").next())
    .and_then(|output| expand(xml::attribute(output, "url")?, macros, root));
    let mut module_files = Vec::new();
    if let Some(manager) = component(&modules_document, "ProjectModuleManager") {
        for module in xml::descendants(manager, "module") {
            let declared = xml::attribute(module, "filepath")
                .or_else(|| xml::attribute(module, "fileurl"))
                .unwrap_or_default();
            if let Some(path) = expand(declared, macros, &macros.project_dir) {
                module_files.push(path);
            }
        }
    }
    if module_files.len() > MAX_MODULES {
        module_files.truncate(MAX_MODULES);
        facts.truncated = true;
    }
    let mut roots = 0usize;
    let mut seen = BTreeSet::new();
    for file in module_files {
        if !seen.insert(file.clone()) {
            continue;
        }
        let Some(document) = read_bounded(&file).and_then(|bytes| xml::parse(&bytes)) else {
            facts.unresolved.push(Unresolved {
                kind: UnresolvedKind::MissingFile,
                name: relative_or_absolute(root, &file),
            });
            continue;
        };
        facts.modules += 1;
        let module_dir = file
            .parent()
            .filter(|path| *path != directory)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        let module_name = file
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("module")
            .to_string();
        let mut module = IdeaFacts::default();
        read_module(
            root,
            &document,
            &module_dir,
            &module_name,
            macros,
            &project_libraries,
            &mut module,
            &mut roots,
        );
        facts.needs_servlet |= module.needs_servlet;
        facts.truncated |= module.truncated;
        append_unique(&mut facts.hints, &module.hints);
        append_unique(&mut facts.web_roots, &module.web_roots);
        append_unique(&mut facts.source_paths, &module.source_paths);
        append_unique(&mut facts.test_source_paths, &module.test_source_paths);
        append_unique(&mut facts.unresolved, &module.unresolved);
        for library in &module.libraries {
            push(&mut facts.libraries, library.clone());
        }
        facts.module_details.push(IdeaModule {
            descriptor: relative_or_absolute(root, &file),
            output: if module.inherits_output {
                project_output.as_ref().map(|base| {
                    relative_or_absolute(root, &base.join("production").join(&module_name))
                })
            } else {
                module.output
            },
            test_output: if module.inherits_output {
                project_output
                    .as_ref()
                    .map(|base| relative_or_absolute(root, &base.join("test").join(&module_name)))
            } else {
                module.test_output
            },
            inherits_output: module.inherits_output,
            name: module_name,
            sources: module.source_paths,
            test_sources: module.test_source_paths,
            content_roots: module.content_roots,
            resources: module.resources,
            test_resources: module.test_resources,
            excluded: module.excluded,
            libraries: module.libraries,
            class_folders: module.class_folders,
            module_dependencies: module.module_dependencies,
            unresolved: module.unresolved,
            sdk: module.sdk,
        });
    }
    Some(facts)
}

#[allow(clippy::too_many_arguments)]
fn read_module(
    root: &Path,
    document: &Element,
    module_dir: &Path,
    module_name: &str,
    macros: &Macros,
    project_libraries: &BTreeMap<String, Element>,
    facts: &mut IdeaFacts,
    roots: &mut usize,
) {
    if let Some(manager) = component(document, "FacetManager") {
        for facet in xml::descendants(manager, "facet") {
            if xml::attribute(facet, "type") == Some("web") {
                facts.needs_servlet = true;
                for web_root in xml::descendants(facet, "root") {
                    if let Some(path) = xml::attribute(web_root, "url")
                        .and_then(|url| expand(url, macros, module_dir))
                    {
                        if !facts.web_roots.contains(&path) {
                            facts.web_roots.push(path);
                        }
                    }
                }
            }
        }
    }
    let Some(manager) = component(document, "NewModuleRootManager") else {
        return;
    };
    let output = |tag| {
        xml::children(manager, tag)
            .next()
            .and_then(|entry| expand(xml::attribute(entry, "url")?, macros, module_dir))
            .map(|path| relative_or_absolute(root, &path))
    };
    facts.output = output("output");
    facts.test_output = output("output-test");
    facts.inherits_output = xml::attribute(manager, "inherit-compiler-output") == Some("true");
    for content in xml::children(manager, "content") {
        if let Some(path) =
            xml::attribute(content, "url").and_then(|url| expand(url, macros, module_dir))
        {
            let path = relative_or_absolute(root, &path);
            if !facts.content_roots.contains(&path) {
                facts.content_roots.push(path);
            }
        }
        for folder in xml::children(content, "excludeFolder") {
            if let Some(path) =
                xml::attribute(folder, "url").and_then(|url| expand(url, macros, module_dir))
            {
                let path = relative_or_absolute(root, &path);
                if !facts.excluded.contains(&path) {
                    facts.excluded.push(path);
                }
            }
        }
        for folder in xml::children(content, "sourceFolder") {
            let Some(path) =
                xml::attribute(folder, "url").and_then(|url| expand(url, macros, module_dir))
            else {
                continue;
            };
            let Some(relative) = relative_inside(root, &path) else {
                facts.unresolved.push(Unresolved {
                    kind: UnresolvedKind::OutsideWorkspace,
                    name: path.to_string_lossy().to_string(),
                });
                continue;
            };
            let target = match xml::attribute(folder, "type") {
                Some("java-resource") => &mut facts.resources,
                Some("java-test-resource") => &mut facts.test_resources,
                Some(kind) if kind.contains("resource") => continue,
                Some("java-test-source") => &mut facts.test_source_paths,
                _ if xml::attribute(folder, "isTestSource") == Some("true") => {
                    &mut facts.test_source_paths
                }
                _ => &mut facts.source_paths,
            };
            if !target.contains(&relative) {
                target.push(relative);
            }
        }
    }
    for entry in xml::children(manager, "orderEntry") {
        match xml::attribute(entry, "type") {
            Some("jdk") => facts.sdk = xml::attribute(entry, "jdkName").map(str::to_owned),
            Some("module") => {
                if let Some(name) = xml::attribute(entry, "module-name") {
                    facts.module_dependencies.push(name.to_owned());
                }
            }
            Some("module-library") => {
                if let Some(library) = xml::children(entry, "library").next() {
                    let name = xml::attribute(library, "name")
                        .unwrap_or(module_name)
                        .to_string();
                    add_library(
                        root,
                        library,
                        &name,
                        Origin::IdeaModuleLibrary,
                        macros,
                        module_dir,
                        facts,
                        roots,
                    );
                }
            }
            Some("library") => {
                let name = xml::attribute(entry, "name")
                    .unwrap_or_default()
                    .to_string();
                match xml::attribute(entry, "level") {
                    Some("project") => match project_libraries.get(&name) {
                        Some(library) => {
                            add_library(
                                root,
                                library,
                                &name,
                                Origin::IdeaProjectLibrary,
                                macros,
                                &macros.project_dir,
                                facts,
                                roots,
                            );
                        }
                        None => facts.unresolved.push(Unresolved {
                            kind: UnresolvedKind::MissingLibrary,
                            name,
                        }),
                    },
                    Some("application_server_libraries") => {
                        facts.needs_servlet = true;
                        if let Some(hint) = ServletHint::from_name(&name) {
                            facts.hints.push(hint);
                        }
                    }
                    _ => {
                        if !facts.unresolved.iter().any(|item| item.name == name) {
                            facts.unresolved.push(Unresolved {
                                kind: UnresolvedKind::IdeLibrary,
                                name,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_library(
    root: &Path,
    library: &Element,
    name: &str,
    origin: Origin,
    macros: &Macros,
    base: &Path,
    facts: &mut IdeaFacts,
    roots: &mut usize,
) {
    let jar_directories: Vec<_> = xml::children(library, "jarDirectory")
        .filter(|entry| xml::attribute(entry, "type") != Some("SOURCES"))
        .filter_map(|entry| {
            let path = expand(xml::attribute(entry, "url")?, macros, base)?;
            Some((path, xml::attribute(entry, "recursive") == Some("true")))
        })
        .collect();
    for classes in xml::children(library, "CLASSES") {
        for entry in xml::children(classes, "root") {
            if *roots >= MAX_LIBRARY_ROOTS {
                facts.truncated = true;
                return;
            }
            *roots += 1;
            let url = xml::attribute(entry, "url").unwrap_or_default();
            let Some(path) = expand(url, macros, base) else {
                facts.unresolved.push(Unresolved {
                    kind: UnresolvedKind::UnknownMacro,
                    name: url.to_string(),
                });
                continue;
            };
            if path.is_dir() {
                if !jar_directories
                    .iter()
                    .any(|(directory, _)| directory == &path)
                {
                    push(
                        &mut facts.class_folders,
                        Detected {
                            pattern: relative_or_absolute(root, &path),
                            origin,
                            label: name.to_string(),
                            count: 1,
                        },
                    );
                }
                continue;
            }
            if !path.is_file() {
                facts.unresolved.push(Unresolved {
                    kind: UnresolvedKind::MissingFile,
                    name: relative_or_absolute(root, &path),
                });
                continue;
            }
            push(
                &mut facts.libraries,
                Detected {
                    pattern: relative_or_absolute(root, &path),
                    origin,
                    label: name.to_string(),
                    count: 1,
                },
            );
        }
    }
    for (path, recursive) in jar_directories {
        let count = super::web::count_libraries(&path);
        if count == 0 && !recursive {
            continue;
        }
        let glob = if recursive { "**/*.jar" } else { "*.jar" };
        push(
            &mut facts.libraries,
            Detected {
                pattern: format!("{}/{glob}", relative_or_absolute(root, &path)),
                origin,
                label: name.to_string(),
                count: count.max(1),
            },
        );
    }
}

fn push(libraries: &mut Vec<Detected>, library: Detected) {
    if !libraries.iter().any(|item| item.pattern == library.pattern) {
        libraries.push(library);
    }
}

fn append_unique<T: Clone + PartialEq>(target: &mut Vec<T>, values: &[T]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}
/// The `<component name="…">` of a module or project document.
fn component<'a>(document: &'a Element, name: &str) -> Option<&'a Element> {
    xml::children(document, "component").find(|item| xml::attribute(item, "name") == Some(name))
}

fn directory_libraries(directory: &Path) -> BTreeMap<String, Element> {
    let mut libraries = BTreeMap::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return libraries;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .collect();
    files.sort();
    files.truncate(MAX_LIBRARY_FILES);
    for file in files {
        if let Some(document) = read_bounded(&file).and_then(|bytes| xml::parse(&bytes)) {
            libraries.extend(named_libraries(&document));
        }
    }
    libraries
}

fn named_libraries(table: &Element) -> BTreeMap<String, Element> {
    xml::descendants(table, "library")
        .into_iter()
        .filter_map(|library| {
            xml::attribute(library, "name").map(|name| (name.to_string(), library.clone()))
        })
        .collect()
}

/// Expands `file://`/`jar://…!/` URLs and the path macros IDEA writes, with
/// `$MODULE_DIR$` meaning `module_dir`.
/// Unknown macros (path variables, the IDE's own folder) give None.
fn expand(value: &str, macros: &Macros, module_dir: &Path) -> Option<PathBuf> {
    let mut text = value
        .trim()
        .trim_start_matches("jar://")
        .trim_start_matches("file://")
        .trim_end_matches("!/")
        .to_string();
    let replacements: [(&str, Option<&Path>); 4] = [
        ("$MODULE_DIR$", Some(module_dir)),
        ("$PROJECT_DIR$", Some(&macros.project_dir)),
        ("$USER_HOME$", macros.user_home.as_deref()),
        ("$MAVEN_REPOSITORY$", macros.maven_repository.as_deref()),
    ];
    for (name, folder) in replacements {
        if text.contains(name) {
            text = text.replace(name, &folder?.to_string_lossy());
        }
    }
    if text.contains('$') || text.is_empty() {
        return None;
    }
    // A Windows drive path keeps its form; IDEA always writes '/'.
    let path = PathBuf::from(text.replace('/', std::path::MAIN_SEPARATOR_STR));
    Some(normalize(&path))
}

/// Removes `.` and `..` components without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::tests::Tree;

    fn macros(tree: &Tree) -> Macros {
        Macros {
            project_dir: tree.root.clone(),
            user_home: Some(tree.root.join("home")),
            maven_repository: Some(tree.root.join("home/.m2/repository")),
        }
    }

    #[test]
    fn modules_source_folders_and_libraries_are_imported_read_only() {
        let tree = Tree::new(&[
            (
                ".idea/misc.xml",
                r#"<project><component name="ProjectRootManager"><output url="file://$PROJECT_DIR$/out"/></component></project>"#,
            ),
            (
                ".idea/modules.xml",
                r#"<project version="4"><component name="ProjectModuleManager"><modules>
                    <module fileurl="file://$PROJECT_DIR$/.idea/app.iml" filepath="$PROJECT_DIR$/.idea/app.iml"/>
                    <module fileurl="file://$PROJECT_DIR$/core/core.iml" filepath="$PROJECT_DIR$/core/core.iml"/>
                    <module fileurl="file://$PROJECT_DIR$/gone.iml" filepath="$PROJECT_DIR$/gone.iml"/>
                </modules></component></project>"#,
            ),
            (
                ".idea/app.iml",
                r#"<module type="JAVA_MODULE" version="4">
                  <component name="FacetManager"><facet type="web" name="Web"><configuration><webroots><root url="file://$MODULE_DIR$/web" relative="/"/></webroots></configuration></facet></component>
                  <component name="NewModuleRootManager" inherit-compiler-output="true">
                    <content url="file://$MODULE_DIR$">
                      <sourceFolder url="file://$MODULE_DIR$/src" isTestSource="false"/>
                      <sourceFolder url="file://$MODULE_DIR$/test" isTestSource="true"/>
                      <sourceFolder url="file://$MODULE_DIR$/resources" type="java-resource"/>
                      <sourceFolder url="file://$MODULE_DIR$/test-resources" type="java-test-resource"/>
                      <excludeFolder url="file://$MODULE_DIR$/generated"/>
                      <sourceFolder url="file://$MODULE_DIR$/../outside" isTestSource="false"/>
                    </content>
                    <orderEntry type="inheritedJdk"/>
                    <orderEntry type="sourceFolder" forTests="false"/>
                    <orderEntry type="module" module-name="core"/>
                    <orderEntry type="module-library"><library><CLASSES>
                      <root url="jar://$MODULE_DIR$/lib/local.jar!/"/>
                      <root url="jar://$MODULE_DIR$/lib/missing.jar!/"/>
                      <root url="file://$MODULE_DIR$/classes-folder"/>
                      <root url="jar://$TOOLS_DIR$/x.jar!/"/>
                    </CLASSES><SOURCES/><jarDirectory url="file://$MODULE_DIR$/web/WEB-INF/lib" recursive="false"/></library></orderEntry>
                    <orderEntry type="library" name="shared" level="project"/>
                    <orderEntry type="library" name="Tomcat 9.0.98" level="application_server_libraries"/>
                    <orderEntry type="library" name="Global Utils" level="application"/>
                    <orderEntry type="library" name="absent" level="project"/>
                  </component></module>"#,
            ),
            (
                "core/core.iml",
                r#"<module type="JAVA_MODULE" version="4"><component name="NewModuleRootManager">
                    <content url="file://$MODULE_DIR$"><sourceFolder url="file://$MODULE_DIR$/java" isTestSource="false"/></content>
                    <orderEntry type="library" name="shared" level="project"/>
                  </component></module>"#,
            ),
            (
                ".idea/libraries/shared.xml",
                r#"<component name="libraryTable"><library name="shared"><CLASSES>
                    <root url="jar://$MAVEN_REPOSITORY$/org/acme/shared/1.0/shared-1.0.jar!/"/>
                  </CLASSES><JAVADOC/><SOURCES/></library></component>"#,
            ),
            ("home/.m2/repository/org/acme/shared/1.0/shared-1.0.jar", ""),
            ("lib/local.jar", ""),
            ("classes-folder/A.class", ""),
            ("web/WEB-INF/lib/w.jar", ""),
        ]);
        let facts = read(&tree.root, &[], &macros(&tree)).unwrap();
        assert_eq!(facts.descriptor, ".idea/modules.xml");
        assert_eq!(facts.modules, 2);
        assert_eq!(facts.source_paths, vec!["src", "core/java"]);
        assert_eq!(facts.test_source_paths, vec!["test"]);
        assert_eq!(facts.module_details[0].descriptor, ".idea/app.iml");
        let app = &facts.module_details[0];
        assert_eq!(app.content_roots, ["."]);
        assert_eq!(app.resources, ["resources"]);
        assert_eq!(app.test_resources, ["test-resources"]);
        assert_eq!(app.excluded, ["generated"]);
        assert_eq!(app.output.as_deref(), Some("out/production/app"));
        assert_eq!(app.test_output.as_deref(), Some("out/test/app"));
        assert!(app.inherits_output);
        assert_eq!(facts.module_details[0].module_dependencies, vec!["core"]);
        assert_eq!(
            facts.module_details[0].class_folders[0].pattern,
            "classes-folder"
        );
        assert_eq!(facts.module_details[1].libraries[0].label, "shared");
        let libraries: Vec<_> = facts
            .libraries
            .iter()
            .map(|item| (item.pattern.as_str(), item.origin, item.label.as_str()))
            .collect();
        let shared = tree
            .root
            .join("home/.m2/repository/org/acme/shared/1.0/shared-1.0.jar");
        assert_eq!(
            libraries,
            vec![
                ("lib/local.jar", Origin::IdeaModuleLibrary, "app"),
                ("web/WEB-INF/lib/*.jar", Origin::IdeaModuleLibrary, "app"),
                (
                    "home/.m2/repository/org/acme/shared/1.0/shared-1.0.jar",
                    Origin::IdeaProjectLibrary,
                    "shared"
                ),
            ]
        );
        assert!(shared.is_file());
        let unresolved: Vec<_> = facts
            .unresolved
            .iter()
            .map(|item| (item.kind, item.name.as_str()))
            .collect();
        assert_eq!(
            unresolved,
            vec![
                (
                    UnresolvedKind::OutsideWorkspace,
                    tree.root
                        .parent()
                        .unwrap()
                        .join("outside")
                        .to_str()
                        .unwrap()
                ),
                (UnresolvedKind::MissingFile, "lib/missing.jar"),
                (UnresolvedKind::UnknownMacro, "jar://$TOOLS_DIR$/x.jar!/"),
                (UnresolvedKind::IdeLibrary, "Global Utils"),
                (UnresolvedKind::MissingLibrary, "absent"),
                (UnresolvedKind::MissingFile, "gone.iml"),
            ]
        );
        assert!(facts.needs_servlet);
        assert_eq!(facts.hints, vec![ServletHint::Major(9)]);
        assert_eq!(facts.web_roots, vec![tree.root.join("web")]);
    }

    #[test]
    fn a_file_based_project_reads_its_ipr() {
        let tree = Tree::new(&[
            (
                "Legacy.ipr",
                r#"<project version="4">
                  <component name="ProjectModuleManager"><modules><module filepath="$PROJECT_DIR$/Legacy.iml"/></modules></component>
                  <component name="libraryTable"><library name="libs"><CLASSES><root url="jar://$PROJECT_DIR$/lib/a.jar!/"/></CLASSES></library></component>
                </project>"#,
            ),
            (
                "Legacy.iml",
                r#"<module><component name="NewModuleRootManager"><content url="file://$MODULE_DIR$"><sourceFolder url="file://$MODULE_DIR$/source" isTestSource="false"/></content><orderEntry type="library" name="libs" level="project"/></component></module>"#,
            ),
            ("lib/a.jar", ""),
        ]);
        let facts = read(&tree.root, &[tree.root.join("Legacy.ipr")], &macros(&tree)).unwrap();
        assert_eq!(facts.descriptor, "Legacy.ipr");
        assert_eq!(facts.source_paths, vec!["source"]);
        assert_eq!(facts.libraries[0].pattern, "lib/a.jar");
        assert!(read(&tree.root.join("lib"), &[], &macros(&tree)).is_none());
    }
}
