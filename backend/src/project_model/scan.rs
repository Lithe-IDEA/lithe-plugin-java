//! One bounded walk of a workspace folder that finds every file the project
//! model readers need. It never follows symbolic links, skips hidden,
//! dependency and build output trees, and reads at most
//! [`SCAN_ENTRY_BUDGET`] entries, so a huge workspace costs a bounded delay.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// How deep below the workspace folder the walk descends. Legacy layouts keep
/// web roots at most a few levels down (`module/WebRoot/WEB-INF`).
pub const SCAN_DEPTH: usize = 6;
/// Most directory entries one walk reads.
pub const SCAN_ENTRY_BUDGET: usize = 20_000;
/// Most items of each kind the walk records.
pub const SCAN_ITEM_LIMIT: usize = 256;
/// Directories never entered: dependency and build output trees, and the
/// descriptor folders whose few known children are read directly.
pub const SKIPPED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "bin",
    "out",
    "dist",
    "vendor",
    "classes",
    "WEB-INF",
    "META-INF",
    "archetype-resources",
];
/// Files whose presence makes JDT LS import the folder with Maven or Gradle.
pub const BUILD_FILES: &[&str] = &[
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];

/// What one walk found, in walk order (breadth first, names sorted).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Maven and Gradle build files.
    pub build_files: Vec<PathBuf>,
    /// Folders holding an Eclipse `.project`.
    pub eclipse_projects: Vec<PathBuf>,
    /// Folders holding a `WEB-INF` directory: deployed web application roots.
    pub web_roots: Vec<PathBuf>,
    /// Exploded EAR roots: folders holding `META-INF/application.xml`, and
    /// WTP's `EarContent` folders.
    pub ear_roots: Vec<PathBuf>,
    /// IntelliJ IDEA file-based project descriptors at the folder's top.
    pub ipr_files: Vec<PathBuf>,
    /// The budget or an item limit stopped the walk early.
    pub truncated: bool,
}

/// Walks `root`; an unreadable directory contributes nothing.
pub fn scan(root: &Path) -> Scan {
    let mut found = Scan::default();
    let mut budget = SCAN_ENTRY_BUDGET;
    let mut pending = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((directory, depth)) = pending.pop_front() {
        // An unreadable directory contributes nothing to the model.
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if budget == 0 {
                found.truncated = true;
                return found;
            }
            budget -= 1;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if name == "WEB-INF" {
                    push(
                        &mut found.web_roots,
                        directory.clone(),
                        &mut found.truncated,
                    );
                } else if name == "META-INF" && path.join("application.xml").is_file() {
                    push(
                        &mut found.ear_roots,
                        directory.clone(),
                        &mut found.truncated,
                    );
                } else if name == "EarContent" {
                    push(&mut found.ear_roots, path.clone(), &mut found.truncated);
                }
                let skipped = name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name);
                if depth < SCAN_DEPTH && !skipped {
                    pending.push_back((path, depth + 1));
                }
            } else if kind.is_file() {
                if BUILD_FILES.contains(&name) {
                    push(&mut found.build_files, path, &mut found.truncated);
                } else if name == ".project" {
                    push(
                        &mut found.eclipse_projects,
                        directory.clone(),
                        &mut found.truncated,
                    );
                } else if depth == 0 && name.ends_with(".ipr") {
                    push(&mut found.ipr_files, path, &mut found.truncated);
                }
            }
        }
    }
    found.ear_roots.sort();
    found.ear_roots.dedup();
    found
}

fn push(items: &mut Vec<PathBuf>, item: PathBuf, truncated: &mut bool) {
    if items.len() >= SCAN_ITEM_LIMIT {
        *truncated = true;
    } else if !items.contains(&item) {
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::tests::Tree;

    #[test]
    fn finds_build_files_eclipse_projects_web_and_ear_roots_but_skips_build_trees() {
        let tree = Tree::new(&[
            ("pom.xml", ""),
            ("legacy/.project", ""),
            ("legacy/WebContent/WEB-INF/lib/a.jar", ""),
            ("ear/EarContent/lib/b.jar", ""),
            ("exploded/META-INF/application.xml", "<application/>"),
            ("target/dep/pom.xml", ""),
            ("target/x/WEB-INF/lib/c.jar", ""),
            ("lib/META-INF/maven/pom.xml", ""),
            (".hidden/pom.xml", ""),
            ("Demo.ipr", ""),
        ]);
        let found = scan(&tree.root);
        assert_eq!(found.build_files, vec![tree.root.join("pom.xml")]);
        assert_eq!(found.eclipse_projects, vec![tree.root.join("legacy")]);
        assert_eq!(found.web_roots, vec![tree.root.join("legacy/WebContent")]);
        assert_eq!(
            found.ear_roots,
            vec![tree.root.join("ear/EarContent"), tree.root.join("exploded")]
        );
        assert_eq!(found.ipr_files, vec![tree.root.join("Demo.ipr")]);
        assert!(!found.truncated);
    }

    #[test]
    fn a_missing_folder_is_an_empty_scan() {
        let tree = Tree::new(&[]);
        assert_eq!(scan(&tree.root.join("missing")), Scan::default());
    }
}
