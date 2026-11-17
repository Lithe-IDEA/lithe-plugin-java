//! Libraries of legacy web and enterprise layouts without a build file:
//! `WEB-INF/lib` of every web root, the library directory of every exploded
//! EAR, and the manifest `Class-Path` of those modules. Only fixed paths below
//! roots the scan found are read.

use super::{read_bounded, relative_or_absolute, xml, Detected, Origin};
use std::fs;
use std::path::{Path, PathBuf};

/// Most JAR files counted in one directory; the count is only displayed.
pub const MAX_COUNTED_LIBRARIES: usize = 4096;

/// Which servlet API a web module's descriptor declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServletNamespace {
    /// `java.sun.com`/`xmlns.jcp.org` descriptors: Java EE, `javax.servlet`.
    Javax,
    /// `jakarta.ee` descriptors: Jakarta EE 9+, `jakarta.servlet`.
    Jakarta,
}

/// What the web and EAR roots contribute.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WebFacts {
    pub libraries: Vec<Detected>,
    /// The namespace of the first web.xml that declares one.
    pub namespace: Option<ServletNamespace>,
}

/// Reads the libraries of `web_roots` and `ear_roots` below `root`.
pub fn read(root: &Path, web_roots: &[PathBuf], ear_roots: &[PathBuf]) -> WebFacts {
    let mut facts = WebFacts::default();
    for web_root in web_roots {
        let lib = web_root.join("WEB-INF/lib");
        add_directory(root, &lib, Origin::WebLibraries, &mut facts.libraries);
        if facts.namespace.is_none() {
            facts.namespace = namespace(&web_root.join("WEB-INF/web.xml"));
        }
    }
    for ear_root in ear_roots {
        let directory = ear_root.join(ear_library_directory(ear_root));
        add_directory(root, &directory, Origin::EarLibraries, &mut facts.libraries);
    }
    // A module's manifest names archives relative to where the module sits
    // when deployed: the EAR's root, or beside the module itself.
    for web_root in web_roots {
        let mut bases: Vec<&Path> = ear_roots.iter().map(PathBuf::as_path).collect();
        if let Some(parent) = web_root.parent() {
            bases.push(parent);
        }
        for entry in manifest_class_path(web_root) {
            for base in &bases {
                let file = base.join(&entry);
                if file.is_file() {
                    let pattern = relative_or_absolute(root, &file);
                    if !facts.libraries.iter().any(|item| item.pattern == pattern) {
                        facts.libraries.push(Detected {
                            pattern,
                            origin: Origin::ManifestClassPath,
                            label: relative_or_absolute(root, web_root),
                            count: 1,
                        });
                    }
                    break;
                }
            }
        }
    }
    facts
}

/// Adds `<directory>/*.jar` when the directory holds at least one JAR.
fn add_directory(root: &Path, directory: &Path, origin: Origin, libraries: &mut Vec<Detected>) {
    let count = count_libraries(directory);
    if count == 0 {
        return;
    }
    let pattern = format!("{}/*.jar", relative_or_absolute(root, directory));
    if !libraries.iter().any(|item| item.pattern == pattern) {
        libraries.push(Detected {
            pattern,
            origin,
            label: relative_or_absolute(root, directory),
            count,
        });
    }
}

/// The number of JAR files directly inside `directory`, at most
/// [`MAX_COUNTED_LIBRARIES`].
pub fn count_libraries(directory: &Path) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".jar"))
        })
        .take(MAX_COUNTED_LIBRARIES)
        .count()
}

/// application.xml's `<library-directory>`, else the Java EE 5 default `lib`.
pub fn ear_library_directory(ear_root: &Path) -> String {
    let declared = read_bounded(&ear_root.join("META-INF/application.xml"))
        .and_then(|bytes| xml::parse(&bytes))
        .and_then(|document| {
            xml::descendants(&document, "library-directory")
                .first()
                .and_then(|element| element.get_text())
                .map(|text| text.trim().trim_matches('/').to_string())
        });
    match declared {
        Some(directory) if !directory.is_empty() && !directory.contains("..") => directory,
        _ => "lib".to_string(),
    }
}

/// The relative `Class-Path` entries of `<module_root>/META-INF/MANIFEST.MF`.
pub fn manifest_class_path(module_root: &Path) -> Vec<String> {
    let Some(bytes) = read_bounded(&module_root.join("META-INF/MANIFEST.MF")) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    // Manifest lines continue on the next line when it starts with a space.
    let mut joined = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            joined.push_str(rest);
        } else {
            joined.push('\n');
            joined.push_str(line);
        }
    }
    joined
        .lines()
        .find_map(|line| line.strip_prefix("Class-Path:"))
        .map(|value| {
            value
                .split_whitespace()
                .filter(|entry| !entry.contains("..") && !entry.contains(':'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn namespace(web_xml: &Path) -> Option<ServletNamespace> {
    let bytes = read_bounded(web_xml)?;
    let text = String::from_utf8_lossy(&bytes);
    if text.contains("jakarta.ee/xml/ns/jakartaee") {
        Some(ServletNamespace::Jakarta)
    } else if text.contains("java.sun.com/xml/ns")
        || text.contains("xmlns.jcp.org/xml/ns")
        || text.contains("java.sun.com/dtd/web-app")
    {
        Some(ServletNamespace::Javax)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::tests::Tree;

    #[test]
    fn web_inf_ear_and_manifest_libraries_are_found_with_counts() {
        let tree = Tree::new(&[
            ("web/WebContent/WEB-INF/lib/a.jar", ""),
            ("web/WebContent/WEB-INF/lib/b.jar", ""),
            ("web/WebContent/WEB-INF/lib/readme.txt", ""),
            (
                "web/WebContent/WEB-INF/web.xml",
                r#"<web-app xmlns="http://xmlns.jcp.org/xml/ns/javaee" version="3.1"/>"#,
            ),
            (
                "web/WebContent/META-INF/MANIFEST.MF",
                "Manifest-Version: 1.0\nClass-Path: shared.jar lib/co\n mmon.jar ../escape.jar\n",
            ),
            (
                "ear/META-INF/application.xml",
                "<application><library-directory>APP-INF/lib</library-directory></application>",
            ),
            ("ear/APP-INF/lib/c.jar", ""),
            ("ear/shared.jar", ""),
            ("web/lib/common.jar", ""),
        ]);
        let facts = read(
            &tree.root,
            &[tree.root.join("web/WebContent")],
            &[tree.root.join("ear")],
        );
        let patterns: Vec<_> = facts
            .libraries
            .iter()
            .map(|item| (item.pattern.as_str(), item.origin, item.count))
            .collect();
        assert_eq!(
            patterns,
            vec![
                ("web/WebContent/WEB-INF/lib/*.jar", Origin::WebLibraries, 2),
                ("ear/APP-INF/lib/*.jar", Origin::EarLibraries, 1),
                ("ear/shared.jar", Origin::ManifestClassPath, 1),
                ("web/lib/common.jar", Origin::ManifestClassPath, 1),
            ]
        );
        assert_eq!(facts.namespace, Some(ServletNamespace::Javax));
    }

    #[test]
    fn an_empty_web_inf_lib_contributes_nothing_and_jakarta_is_recognized() {
        let tree = Tree::new(&[
            ("WebRoot/WEB-INF/lib/.keep", ""),
            (
                "WebRoot/WEB-INF/web.xml",
                r#"<web-app xmlns="https://jakarta.ee/xml/ns/jakartaee" version="6.0"/>"#,
            ),
        ]);
        let facts = read(&tree.root, &[tree.root.join("WebRoot")], &[]);
        assert!(facts.libraries.is_empty());
        assert_eq!(facts.namespace, Some(ServletNamespace::Jakarta));
    }
}
