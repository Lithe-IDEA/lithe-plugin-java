//! Facts from Eclipse `.classpath` files that the backend needs before JDT LS
//! has imported anything: whether a servlet container must be configured and
//! which servlet container version the project names. Resolving the
//! containers themselves is the job of the JDT LS extension bundle.

use super::{read_bounded, servlet::ServletHint, xml};
use std::path::{Path, PathBuf};

/// Container IDs (first path segment) the bundle fills from the servlet
/// container. Must match `ContainerKind` of the bundle.
pub const SERVLET_CONTAINERS: &[&str] = &[
    "org.eclipse.jst.server.core.container",
    "com.genuitec.eclipse.j2eedt.core.J2EE13_CONTAINER",
    "com.genuitec.eclipse.j2eedt.core.J2EE14_CONTAINER",
    "melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_5_CONTAINER",
    "melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_6_CONTAINER",
    "melibrary.com.genuitec.eclipse.j2eedt.core.MYECLIPSE_JAVAEE_7_CONTAINER",
    "com.genuitec.runtime.library",
];
/// Classpath variables the bundle binds to the servlet container's folder.
pub const SERVLET_VARIABLES: &[&str] = &["TOMCAT_HOME", "CATALINA_HOME"];
/// User libraries live in an Eclipse workspace, not in the project.
pub const USER_LIBRARY: &str = "org.eclipse.jdt.USER_LIBRARY";

/// What the `.classpath` files of the scanned Eclipse projects say.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EclipseFacts {
    pub needs_servlet: bool,
    pub hints: Vec<ServletHint>,
    /// Names of user libraries the projects reference.
    pub user_libraries: Vec<String>,
}

pub fn read(projects: &[PathBuf]) -> EclipseFacts {
    let mut facts = EclipseFacts::default();
    for project in projects {
        read_project(project, &mut facts);
    }
    facts
}

fn read_project(project: &Path, facts: &mut EclipseFacts) {
    let Some(document) =
        read_bounded(&project.join(".classpath")).and_then(|bytes| xml::parse(&bytes))
    else {
        return;
    };
    for entry in xml::children(&document, "classpathentry") {
        let path = xml::attribute(entry, "path").unwrap_or_default();
        let mut segments = path.split('/');
        let first = segments.next().unwrap_or_default();
        match xml::attribute(entry, "kind") {
            Some("con") if SERVLET_CONTAINERS.contains(&first) => {
                facts.needs_servlet = true;
                if let Some(hint) = path.rsplit('/').next().and_then(ServletHint::from_name) {
                    facts.hints.push(hint);
                }
            }
            Some("con") if first == USER_LIBRARY => {
                if let Some(name) = segments.next().filter(|name| !name.is_empty()) {
                    if !facts.user_libraries.iter().any(|known| known == name) {
                        facts.user_libraries.push(name.to_string());
                    }
                }
            }
            Some("var") if SERVLET_VARIABLES.contains(&first) => facts.needs_servlet = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::tests::Tree;

    #[test]
    fn server_runtime_containers_variables_and_user_libraries_are_recognized() {
        let tree = Tree::new(&[
            (
                "app/.classpath",
                r#"<classpath>
                  <classpathentry kind="src" path="src"/>
                  <classpathentry kind="con" path="org.eclipse.jst.server.core.container/org.eclipse.jst.server.tomcat.runtimeTarget/Apache Tomcat v8.5"/>
                  <classpathentry kind="con" path="org.eclipse.jdt.USER_LIBRARY/Shared Libs"/>
                  <classpathentry kind="con" path="org.eclipse.jdt.USER_LIBRARY/Shared Libs"/>
                </classpath>"#,
            ),
            (
                "tool/.classpath",
                r#"<classpath><classpathentry kind="var" path="TOMCAT_HOME/lib/servlet-api.jar"/></classpath>"#,
            ),
            (
                "plain/.classpath",
                r#"<classpath><classpathentry kind="lib" path="a.jar"/></classpath>"#,
            ),
        ]);
        let facts = read(&[tree.root.join("plain")]);
        assert!(!facts.needs_servlet);
        let facts = read(&[
            tree.root.join("app"),
            tree.root.join("tool"),
            tree.root.join("missing"),
        ]);
        assert!(facts.needs_servlet);
        assert_eq!(facts.hints, vec![ServletHint::Major(8)]);
        assert_eq!(facts.user_libraries, vec!["Shared Libs"]);
    }
}
