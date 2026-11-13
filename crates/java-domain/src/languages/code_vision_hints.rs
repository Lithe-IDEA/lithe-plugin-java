//! Code Vision hints with usage counts and the last author of each line.
//!
//! Ported from the Swift `JavaCodeVisionService`: project files are mapped to
//! workspace-relative paths, usage counts come from [`code_vision`], and the
//! blame author of the declaration line is attached.
//!
//! TODO(host-api: API-42 built-in Git read-only): blame lines come from the
//! host Git service. The backend needs a read-only `git.blame(file)` API; until
//! then callers pass the lines the host already computed, or none.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{code_vision, JavaCodeVisionRequest};
use crate::testing::workflow::normalize;

/// The author of one source line, as reported by Git blame.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    /// Zero-based line, matching [`JavaCodeVisionHint::line`].
    pub line: usize,
    /// Author of the last change to the line.
    pub author_name: String,
}

/// Hint rendered above one Java declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaCodeVisionHint {
    /// Zero-based declaration line.
    pub line: usize,
    /// UTF-16 column of the declaration name.
    pub utf16_column: usize,
    /// Declared symbol.
    pub symbol: String,
    /// Workspace references to the symbol.
    pub usage_count: usize,
    /// Implementations; the source scan does not count them.
    pub implementation_count: usize,
    /// Last author of the declaration line, when blame is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

/// `file` relative to `root`, or `None` when it is not inside the root.
pub fn workspace_relative_path(file: &Path, root: &Path) -> Option<String> {
    let file = normalize(file);
    let root = normalize(root);
    let relative = file.strip_prefix(&root).ok()?;
    let relative = relative.to_str()?;
    (!relative.is_empty()).then(|| relative.replace(std::path::MAIN_SEPARATOR, "/"))
}

/// Hints for `file`; a file outside the workspace or a failed scan has none.
pub fn code_vision_hints(
    file: &Path,
    project_files: &[impl AsRef<Path>],
    workspace_root: &Path,
    blame_lines: &[BlameLine],
) -> Vec<JavaCodeVisionHint> {
    let Some(target_path) = workspace_relative_path(file, workspace_root) else {
        return Vec::new();
    };
    let paths = project_files
        .iter()
        .filter_map(|path| workspace_relative_path(path.as_ref(), workspace_root))
        .collect();
    let Ok(response) = code_vision(JavaCodeVisionRequest {
        root: normalize(workspace_root).display().to_string(),
        target_path,
        paths,
    }) else {
        return Vec::new();
    };
    let authors = blame_lines
        .iter()
        .map(|blame| (blame.line, blame.author_name.as_str()))
        .collect::<HashMap<_, _>>();
    response
        .hints
        .into_iter()
        .map(|hint| JavaCodeVisionHint {
            author_name: authors.get(&hint.line).map(|name| (*name).to_string()),
            line: hint.line,
            utf16_column: hint.utf16_column,
            symbol: hint.symbol,
            usage_count: hint.usage_count,
            implementation_count: 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn relative_paths_stay_inside_the_workspace() {
        let root = Path::new("/w/project");
        assert_eq!(
            workspace_relative_path(Path::new("/w/project/src/./A.java"), root),
            Some("src/A.java".into())
        );
        assert_eq!(
            workspace_relative_path(Path::new("/w/project-2/A.java"), root),
            None
        );
        assert_eq!(workspace_relative_path(root, root), None);
    }

    #[test]
    fn hints_carry_usage_counts_and_blame_authors() {
        let root =
            std::env::temp_dir().join(format!("lithe-java-code-vision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let temp = TempDir(root.clone());
        fs::create_dir_all(root.join("src")).unwrap();
        let service = root.join("src/UserService.java");
        let caller = root.join("src/Caller.java");
        fs::write(&service, "class UserService {\n    void save() {}\n}\n").unwrap();
        fs::write(
            &caller,
            "class Caller {\n    void run(UserService service) { service.save(); }\n}\n",
        )
        .unwrap();
        let outside = PathBuf::from("/elsewhere/Other.java");
        let hints = code_vision_hints(
            &service,
            &[service.clone(), caller, outside.clone()],
            &temp.0,
            &[BlameLine {
                line: 1,
                author_name: "Ada".into(),
            }],
        );
        let save = hints
            .iter()
            .find(|hint| hint.symbol == "save")
            .expect("save hint");
        assert_eq!(save.line, 1);
        assert_eq!(save.usage_count, 1);
        assert_eq!(save.implementation_count, 0);
        assert_eq!(save.author_name.as_deref(), Some("Ada"));
        assert!(code_vision_hints(&outside, &[service], &temp.0, &[]).is_empty());
    }
}
