//! The servlet container whose libraries stand in for the "Server Runtime" a
//! web project was written against. Precedence: the plugin setting, then
//! `CATALINA_HOME`/`TOMCAT_HOME`, then installations in the standard places,
//! choosing the one closest to what the project names (an Eclipse runtime
//! "Apache Tomcat v8.5", an IDEA "Tomcat 9.0.98" library, a javax or jakarta
//! web.xml). Only directory listings and two small text files are read.

use super::read_bounded;
use super::web::ServletNamespace;
use std::fs;
use std::path::{Path, PathBuf};

/// Most libraries taken from one installation.
pub const MAX_RUNTIME_LIBRARIES: usize = 256;
/// Environment variables naming a servlet container folder, in order.
pub const HOME_VARIABLES: &[&str] = &["CATALINA_HOME", "TOMCAT_HOME"];
/// Folder name prefixes of servlet container installations.
const INSTALLATION_PREFIXES: &[&str] = &["apache-tomcat", "tomcat", "jetty"];

/// A preference for one installation over another, read from the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServletHint {
    /// The Tomcat major version the project names.
    Major(u32),
    /// The API namespace the project's descriptors or containers imply.
    Namespace(ServletNamespace),
}

impl ServletHint {
    /// The hint in a runtime or library name: "Apache Tomcat v8.5" is Tomcat
    /// 8, "MYECLIPSE_JAVAEE_6_CONTAINER" is javax.
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if let Some(index) = lower.find("tomcat") {
            let digits: String = lower[index + 6..]
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(major) = digits.parse() {
                return Some(Self::Major(major));
            }
        }
        if lower.contains("javaee") || lower.contains("j2ee") || lower.contains("generic_") {
            return Some(Self::Namespace(ServletNamespace::Javax));
        }
        None
    }
}

/// Where the installation in use came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServletSource {
    /// The plugin's servlet container setting.
    Setting,
    /// One of [`HOME_VARIABLES`].
    Environment(&'static str),
    /// Found in a standard installation folder.
    Installed,
}

/// A servlet container installation and the libraries it provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServletRuntime {
    pub home: PathBuf,
    pub libraries: Vec<PathBuf>,
    /// Display name such as "Apache Tomcat 9.0.98".
    pub name: String,
    pub version: Option<String>,
    pub source: ServletSource,
}

/// The outcome of resolving the servlet container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServletResolution {
    pub runtime: Option<ServletRuntime>,
    /// The setting names a folder that is not a servlet container; nothing
    /// else is tried, so the user sees their choice is not in use.
    pub setting_invalid: bool,
}

/// What the resolution may read: environment variables and the folders
/// searched for installations. Tests supply their own.
pub struct ServletEnvironment {
    pub variables: Vec<(&'static str, PathBuf)>,
    /// Folders whose children named like an installation are candidates,
    /// each with the subfolder holding the installation (Homebrew's `libexec`).
    pub search: Vec<(PathBuf, Option<&'static str>)>,
}

impl ServletEnvironment {
    /// The process environment and the standard folders of `os`.
    pub fn current(os: &str) -> Self {
        let variables = HOME_VARIABLES
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (*name, PathBuf::from(value))))
            .filter(|(_, path)| path.is_absolute())
            .collect();
        let home = std::env::var_os(if os == "windows" {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(PathBuf::from);
        let mut search: Vec<(PathBuf, Option<&'static str>)> = match os {
            "macos" => vec![
                (PathBuf::from("/opt/homebrew/opt"), Some("libexec")),
                (PathBuf::from("/usr/local/opt"), Some("libexec")),
                (PathBuf::from("/Library"), None),
                (PathBuf::from("/usr/local"), None),
                (PathBuf::from("/opt"), None),
            ],
            "windows" => vec![
                (
                    PathBuf::from(r"C:\Program Files\Apache Software Foundation"),
                    None,
                ),
                (PathBuf::from(r"C:\"), None),
            ],
            _ => vec![
                (PathBuf::from("/usr/share"), None),
                (PathBuf::from("/opt"), None),
                (PathBuf::from("/usr/local"), None),
            ],
        };
        if let Some(home) = home {
            search.push((home, None));
        }
        Self { variables, search }
    }
}

/// Resolves the servlet container for a project with `hints`.
pub fn resolve(
    setting: Option<&Path>,
    hints: &[ServletHint],
    environment: &ServletEnvironment,
) -> ServletResolution {
    if let Some(folder) = setting {
        return match inspect(folder, ServletSource::Setting) {
            Some(runtime) => ServletResolution {
                runtime: Some(runtime),
                setting_invalid: false,
            },
            None => ServletResolution {
                runtime: None,
                setting_invalid: true,
            },
        };
    }
    for (name, folder) in &environment.variables {
        if let Some(runtime) = inspect(folder, ServletSource::Environment(name)) {
            return ServletResolution {
                runtime: Some(runtime),
                setting_invalid: false,
            };
        }
    }
    let mut installed = Vec::new();
    for (folder, suffix) in &environment.search {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        let mut names: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        names.sort();
        for path in names {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !INSTALLATION_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
            {
                continue;
            }
            let home = suffix.map_or_else(|| path.clone(), |suffix| path.join(suffix));
            if let Some(runtime) = inspect(&home, ServletSource::Installed) {
                if !installed
                    .iter()
                    .any(|known: &ServletRuntime| known.home == runtime.home)
                {
                    installed.push(runtime);
                }
            }
        }
    }
    let best = installed.into_iter().max_by(|left, right| {
        score(left, hints)
            .cmp(&score(right, hints))
            // Equal scores keep the first found, which is the most standard place.
            .then(std::cmp::Ordering::Greater)
    });
    ServletResolution {
        runtime: best,
        setting_invalid: false,
    }
}

/// Orders installations: the named major version, then the implied
/// namespace, then the newest version.
fn score(runtime: &ServletRuntime, hints: &[ServletHint]) -> (bool, bool, Vec<u32>) {
    let version: Vec<u32> = runtime
        .version
        .as_deref()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect();
    let major = version.first().copied();
    let tomcat = runtime.name.contains("Tomcat");
    let exact = hints
        .iter()
        .any(|hint| matches!(hint, ServletHint::Major(wanted) if tomcat && Some(*wanted) == major));
    let namespace = if tomcat && major.is_some_and(|major| major >= 10)
        || runtime
            .libraries
            .iter()
            .any(|library| library.to_string_lossy().contains("jakarta"))
    {
        ServletNamespace::Jakarta
    } else {
        ServletNamespace::Javax
    };
    // The namespace web.xml declares outranks one implied by a version name.
    let wanted = hints
        .iter()
        .find_map(|hint| match hint {
            ServletHint::Namespace(namespace) => Some(*namespace),
            ServletHint::Major(_) => None,
        })
        .or_else(|| {
            hints.iter().find_map(|hint| match hint {
                ServletHint::Major(major) if *major >= 10 => Some(ServletNamespace::Jakarta),
                ServletHint::Major(_) => Some(ServletNamespace::Javax),
                ServletHint::Namespace(_) => None,
            })
        });
    (
        exact,
        wanted.is_none_or(|wanted| wanted == namespace),
        version,
    )
}

/// The installation at `home`, if its `lib` (Tomcat 5.5: `common/lib`) holds
/// a servlet API library.
pub fn inspect(home: &Path, source: ServletSource) -> Option<ServletRuntime> {
    let mut libraries = Vec::new();
    for directory in ["lib", "common/lib"] {
        let Ok(entries) = fs::read_dir(home.join(directory)) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
            })
            .collect();
        found.sort();
        libraries.extend(found);
    }
    libraries.truncate(MAX_RUNTIME_LIBRARIES);
    let has_api = libraries.iter().any(|library| {
        library
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("servlet-api") || name.contains("servlet.api"))
    });
    if !has_api {
        return None;
    }
    let (name, version) = describe(home);
    Some(ServletRuntime {
        home: home.to_path_buf(),
        libraries,
        name,
        version,
        source,
    })
}

/// The display name and version: Tomcat's RELEASE-NOTES, Jetty's
/// VERSION.txt, else the version in the folder's name.
fn describe(home: &Path) -> (String, Option<String>) {
    if let Some(bytes) = read_bounded(&home.join("RELEASE-NOTES")) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(version) = text
            .find("Apache Tomcat Version")
            .and_then(|index| leading_version(&text[index + 21..]))
        {
            return (format!("Apache Tomcat {version}"), Some(version));
        }
    }
    if let Some(bytes) = read_bounded(&home.join("VERSION.txt")) {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(version) = text
            .find("jetty-")
            .and_then(|index| leading_version(&text[index + 6..]))
        {
            return (format!("Eclipse Jetty {version}"), Some(version));
        }
    }
    let folder = home
        .components()
        .rev()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .find(|part| part != "libexec")
        .unwrap_or_default();
    let version = leading_version(&folder);
    let product = if folder.to_ascii_lowercase().contains("jetty") {
        "Eclipse Jetty"
    } else {
        "Apache Tomcat"
    };
    match version {
        Some(version) => (format!("{product} {version}"), Some(version)),
        None => (folder, None),
    }
}

/// The first dotted number in `text`, such as "9.0.98".
fn leading_version(text: &str) -> Option<String> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let version: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let version = version.trim_end_matches('.').to_string();
    (!version.is_empty()).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::tests::Tree;

    fn tomcat(tree: &Tree, folder: &str, version: &str) -> PathBuf {
        tree.write(&format!("{folder}/lib/servlet-api.jar"), "");
        tree.write(&format!("{folder}/lib/catalina.jar"), "");
        tree.write(
            &format!("{folder}/RELEASE-NOTES"),
            &format!("  Apache Tomcat Version {version}\n  Release Notes\n"),
        );
        tree.root.join(folder)
    }

    #[test]
    fn the_setting_wins_and_an_invalid_setting_is_reported_without_falling_back() {
        let tree = Tree::new(&[]);
        let chosen = tomcat(&tree, "chosen", "8.5.100");
        let environment = ServletEnvironment {
            variables: vec![("CATALINA_HOME", tomcat(&tree, "env", "9.0.1"))],
            search: vec![],
        };
        let resolved = resolve(Some(&chosen), &[], &environment);
        let runtime = resolved.runtime.unwrap();
        assert_eq!(
            (runtime.name.as_str(), runtime.source),
            ("Apache Tomcat 8.5.100", ServletSource::Setting)
        );
        assert_eq!(
            runtime.libraries,
            vec![
                chosen.join("lib/catalina.jar"),
                chosen.join("lib/servlet-api.jar")
            ]
        );
        let invalid = resolve(Some(&tree.root.join("missing")), &[], &environment);
        assert_eq!(
            invalid,
            ServletResolution {
                runtime: None,
                setting_invalid: true
            }
        );
        let from_environment = resolve(None, &[], &environment).runtime.unwrap();
        assert_eq!(
            from_environment.source,
            ServletSource::Environment("CATALINA_HOME")
        );
    }

    #[test]
    fn installations_are_chosen_by_named_major_then_namespace_then_newest() {
        let tree = Tree::new(&[]);
        tomcat(&tree, "opt/tomcat@9/libexec", "9.0.98");
        tomcat(&tree, "opt/tomcat@10/libexec", "10.1.30");
        tomcat(&tree, "opt/tomcat/libexec", "11.0.2");
        tree.write("opt/tomcat-broken/libexec/lib/other.jar", "");
        tree.write("opt/unrelated/lib/servlet-api.jar", "");
        let environment = ServletEnvironment {
            variables: vec![],
            search: vec![(tree.root.join("opt"), Some("libexec"))],
        };
        let pick = |hints: &[ServletHint]| {
            resolve(None, hints, &environment)
                .runtime
                .unwrap()
                .version
                .unwrap()
        };
        assert_eq!(pick(&[]), "11.0.2");
        assert_eq!(pick(&[ServletHint::Major(10)]), "10.1.30");
        assert_eq!(
            pick(&[ServletHint::Namespace(ServletNamespace::Javax)]),
            "9.0.98"
        );
        assert_eq!(pick(&[ServletHint::Major(8)]), "9.0.98");
        assert_eq!(
            resolve(
                None,
                &[],
                &ServletEnvironment {
                    variables: vec![],
                    search: vec![]
                }
            ),
            ServletResolution::default()
        );
    }

    #[test]
    fn names_carry_hints() {
        assert_eq!(
            ServletHint::from_name("Apache Tomcat v8.5"),
            Some(ServletHint::Major(8))
        );
        assert_eq!(
            ServletHint::from_name("Tomcat 10.1.30"),
            Some(ServletHint::Major(10))
        );
        assert_eq!(
            ServletHint::from_name("MYECLIPSE_JAVAEE_6_CONTAINER"),
            Some(ServletHint::Namespace(ServletNamespace::Javax))
        );
        assert_eq!(ServletHint::from_name("Shared Libs"), None);
    }

    #[test]
    fn jetty_and_folder_names_describe_installations() {
        let tree = Tree::new(&[
            ("jetty/lib/servlet-api-3.1.jar", ""),
            (
                "jetty/VERSION.txt",
                "jetty-9.4.54.v20240208 - 08 February 2024\n",
            ),
            ("apache-tomcat-7.0.109/lib/servlet-api.jar", ""),
            ("old/common/lib/servlet-api.jar", ""),
        ]);
        assert_eq!(
            inspect(&tree.root.join("jetty"), ServletSource::Installed)
                .unwrap()
                .name,
            "Eclipse Jetty 9.4.54"
        );
        assert_eq!(
            inspect(
                &tree.root.join("apache-tomcat-7.0.109"),
                ServletSource::Installed
            )
            .unwrap()
            .name,
            "Apache Tomcat 7.0.109"
        );
        assert!(inspect(&tree.root.join("old"), ServletSource::Installed).is_some());
    }
}
