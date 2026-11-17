//! The Java libraries settings page beyond the native sources it edits: the
//! detected project model, what Lithe adds in memory, the servlet container,
//! and what JDT made of Eclipse containers. These rows are read-only sources
//! bound to the model's own descriptor files (`.idea/modules.xml`,
//! `.project`, `WEB-INF/web.xml`), so the host's revision checks still hold.
//! The `.vscode/settings.json` source gains source folders and an explicit
//! "save detected entries" choice; nothing is written without Save.

use crate::project_model::servlet::ServletSource;
use crate::project_model::{
    Explicit, ModelKind, Origin, ProjectModel, UnresolvedKind, UPSTREAM_DEFAULT_LIBRARIES,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Field IDs of the `.vscode/settings.json` source.
pub const LIBRARIES_FIELD: &str = "libraries";
pub const SOURCE_PATHS_FIELD: &str = "source-paths";
pub const STORE_DETECTED_FIELD: &str = "store-detected";
/// JDT LS project setting with the absolute source folders of a project.
pub const SOURCE_PATHS_FACT: &str = "org.eclipse.jdt.ls.core.sourcePaths";
/// The project model bundle's command (`DescribeCommandHandler`).
pub const DESCRIBE_COMMAND: &str = "lithe.java.projectModel.describe";
/// Most lines of one read-only list; the host shows at most 256.
const MAX_LINES: usize = 256;
/// The contract's bound of an effective-source label.
const MAX_LABEL_BYTES: usize = 1024;
/// The contract's bound of one list item.
const MAX_ITEM_BYTES: usize = 4096;

fn text(zh: bool, en: &str, cn: &str) -> String {
    if zh { cn } else { en }.to_owned()
}

/// What the page knows when it projects the model.
pub struct Page<'a> {
    pub zh: bool,
    pub root: &'a Path,
    pub model: &'a ProjectModel,
    pub explicit: &'a Explicit,
    /// The bundle's describe answer, when JDT LS gave one.
    pub described: Option<&'a Value>,
}

impl Page<'_> {
    /// Detected includes that may be saved to a shared file: everything but
    /// the machine's servlet container.
    pub fn storable(&self) -> Vec<String> {
        if !self.model.enabled || self.model.kind.managed() {
            return Vec::new();
        }
        self.model
            .libraries
            .iter()
            .map(|item| item.pattern.clone())
            .collect()
    }

    /// IDEA source folders a save would write when none are explicit.
    fn storable_source_paths(&self) -> &[String] {
        if self.model.enabled && self.model.kind == ModelKind::Idea {
            &self.model.source_paths
        } else {
            &[]
        }
    }

    /// Whether the file already holds every storable entry.
    fn stored(&self) -> bool {
        let include = self.explicit.include();
        self.explicit.libraries.is_some()
            && self
                .storable()
                .iter()
                .all(|pattern| include.contains(pattern))
            && (self.storable_source_paths().is_empty() || self.explicit.source_paths.is_some())
    }

    /// The fields of the unmanaged folder's `.vscode/settings.json` source.
    /// `fact` is JDT's project settings answer for the folder.
    pub fn settings_fields(&self, fact: &Value, libraries: Value) -> Vec<Value> {
        let zh = self.zh;
        let detected = crate::project_model::detected_include(self.model).len();
        let library_source = if detected > 0 {
            if zh {
                format!("JDT 实际生效，含自动识别的 {detected} 项")
            } else {
                format!("Effective in JDT, including {detected} detected entries")
            }
        } else {
            text(
                zh,
                "Eclipse JDT project settings",
                "Eclipse JDT 项目生效配置",
            )
        };
        let mut fields = vec![json!({
            "id": LIBRARIES_FIELD,
            "title": text(zh, "Referenced JAR files", "引用的 JAR 文件"),
            "description": text(zh,
                "java.project.referencedLibraries in .vscode/settings.json; lib/** is the upstream default. Lithe also adds the libraries listed under Project model in memory, without writing this file.",
                "写入 .vscode/settings.json 的 java.project.referencedLibraries，上游默认引用 lib/**。Lithe 还会在内存中加入“工程模型”里列出的库，无需写入此文件。"),
            "kind": "fileList",
            "value": libraries,
            "effectiveValue": fact[crate::project_settings::LIBRARIES_FACT].clone(),
            "effectiveSource": library_source,
        })];
        let explicit_sources = self.explicit.source_paths.clone().filter(Value::is_array);
        let inherited = self.storable_source_paths();
        let effective_source = if explicit_sources.is_some() {
            ".vscode/settings.json".to_string()
        } else if !inherited.is_empty() {
            text(zh, "IntelliJ IDEA modules", "IntelliJ IDEA 模块")
        } else {
            text(
                zh,
                "Inferred by JDT from package declarations",
                "JDT 按 package 声明推断",
            )
        };
        fields.push(json!({
            "id": SOURCE_PATHS_FIELD,
            "title": text(zh, "Source folders", "源码文件夹"),
            "description": text(zh,
                "java.project.sourcePaths of a folder without a build file. Empty uses the IntelliJ IDEA modules' source folders, else JDT infers them from the package declarations of Java files. Saved to .vscode/settings.json; JDT applies a change after the language server restarts.",
                "无构建文件的文件夹的源码根（java.project.sourcePaths）。留空时使用 IntelliJ IDEA 模块中的源码文件夹，否则由 JDT 根据 Java 文件的 package 声明推断。保存到 .vscode/settings.json；修改在语言服务器重启后生效。"),
            "kind": "stringList",
            "value": explicit_sources.unwrap_or(Value::Null),
            "effectiveValue": relative_list(self.root, &fact[SOURCE_PATHS_FACT]),
            "inheritedValue": if inherited.is_empty() { Value::Null } else { json!(inherited) },
            "effectiveSource": effective_source,
        }));
        if !self.storable().is_empty() || !self.storable_source_paths().is_empty() {
            let stored = self.stored();
            fields.push(json!({
                "id": STORE_DETECTED_FIELD,
                "title": text(zh, "Save detected entries to this file", "将识别结果保存到此文件"),
                "description": text(zh,
                    "Checked and saved, writes the detected libraries (not this machine's servlet container) and IntelliJ IDEA source folders into .vscode/settings.json, so other editors and teammates get the same classpath. Unchecked and saved, removes them again. Lithe uses them either way.",
                    "勾选并保存后，把自动识别的库（不含本机 Servlet 容器路径）和 IntelliJ IDEA 源码文件夹写入 .vscode/settings.json，供 VS Code 等其他编辑器和团队成员使用；取消勾选并保存则移除这些条目。无论是否保存，Lithe 都会使用它们。"),
                "kind": "boolean",
                "value": stored,
                "effectiveValue": stored,
                "effectiveSource": ".vscode/settings.json",
            }));
        }
        fields
    }

    /// The effective-source label of an Eclipse project's `.classpath`.
    pub fn classpath_label(&self, project: &Path) -> String {
        let zh = self.zh;
        let mut label = text(
            zh,
            "Eclipse JDT project settings",
            "Eclipse JDT 项目生效配置",
        );
        let lines = self.container_lines(project, true);
        if !lines.is_empty() {
            label.push_str(if zh { "；容器：" } else { "; containers: " });
            label.push_str(&lines.join(if zh { "，" } else { ", " }));
        }
        truncate(label, MAX_LABEL_BYTES)
    }

    /// Read-only sources of the model. `taken` holds the paths of the
    /// writable sources, which a read-only row must not reuse.
    pub fn model_sources(&self, taken: &[PathBuf]) -> Vec<(String, PathBuf, Vec<Value>)> {
        let zh = self.zh;
        let mut sources = Vec::new();
        let Some(descriptor) = &self.model.descriptor else {
            return sources;
        };
        let path = self.root.join(descriptor);
        if taken.contains(&path) {
            return sources;
        }
        let mut fields = vec![read_only(
            zh,
            "model",
            text(zh, "Project model", "工程模型"),
            text(zh,
                "Who provides the classpath, in this order: Maven or Gradle build files, Eclipse .project files, an IntelliJ IDEA .idea folder, legacy web folders (WEB-INF/lib), then a plain folder. Detection only reads project files. Turn it off with Project model detection in the Java settings.",
                "按以下顺序决定由谁提供类路径：Maven/Gradle 构建文件、Eclipse .project、IntelliJ IDEA 的 .idea、传统 Web 目录（WEB-INF/lib）、普通文件夹。识别只读取项目文件。可在 Java 设置的“自动识别工程模型”中关闭。"),
            json!(self.model_title()),
            "text",
        )];
        let detected: Vec<String> = self.detected_lines();
        if !detected.is_empty() {
            fields.push(read_only(
                zh,
                "detected",
                text(zh, "Detected libraries", "自动识别的库"),
                text(zh,
                    "Lithe adds these to the classpath in memory; project files stay unchanged. To share them with other editors, check Save detected entries to this file under Referenced libraries.",
                    "Lithe 在内存中把这些库加入类路径，不修改项目文件。若要与其他编辑器共享，在“引用的库”中勾选“将识别结果保存到此文件”。"),
                json!(detected),
                "stringList",
            ));
        }
        if self.model.enabled
            && self.model.kind == ModelKind::Idea
            && !self.model.source_paths.is_empty()
        {
            fields.push(read_only(
                zh,
                "source-folders",
                text(zh, "IntelliJ IDEA source folders", "IntelliJ IDEA 源码文件夹"),
                text(zh,
                    "Production and test source folders of the IDEA modules. JDT treats test folders as ordinary source folders in a folder without a build file.",
                    "IDEA 模块的生产与测试源码文件夹。无构建文件时 JDT 把测试文件夹当作普通源码文件夹处理。"),
                json!(self.model.source_paths.iter().take(MAX_LINES).collect::<Vec<_>>()),
                "stringList",
            ));
        }
        if self.model.needs_servlet {
            fields.push(read_only(
                zh,
                "servlet",
                text(zh, "Servlet container", "Servlet 容器"),
                text(zh,
                    "Web projects get javax.servlet and the other server APIs from a servlet container, as Eclipse's Server Runtime and IDEA's application server libraries do. Automatic uses the Servlet container folder of the Java settings, then CATALINA_HOME or TOMCAT_HOME, then standard installation folders, preferring the version the project names.",
                    "Web 项目的 javax.servlet 等服务器 API 由 Servlet 容器提供，对应 Eclipse 的 Server Runtime 和 IDEA 的应用服务器库。自动：依次使用 Java 设置中的“Servlet 容器文件夹”、CATALINA_HOME 或 TOMCAT_HOME、常见安装位置，并优先选择项目指定的版本。"),
                json!(self.servlet_text()),
                "text",
            ));
        }
        let unresolved = self.unresolved_lines();
        if !unresolved.is_empty() {
            fields.push(read_only(
                zh,
                "unresolved",
                text(zh, "Not resolved", "未能解析"),
                text(zh,
                    "Libraries the project declares but Lithe cannot add. Add their JAR files under Referenced JAR files.",
                    "项目声明了但 Lithe 无法加入的库。请在“引用的 JAR 文件”中添加对应的 JAR。"),
                json!(unresolved),
                "stringList",
            ));
        }
        sources.push(("model".to_string(), path, fields));
        sources
    }

    /// Read-only rows of Eclipse projects with containers, variables or
    /// missing libraries, bound to each project's `.project`.
    pub fn eclipse_sources(&self, taken: &[PathBuf]) -> Vec<(String, PathBuf, Vec<Value>)> {
        let zh = self.zh;
        let mut sources = Vec::new();
        for project in self.described_projects() {
            let Some(location) = project["location"].as_str().map(PathBuf::from) else {
                continue;
            };
            let location = dunce::canonicalize(&location).unwrap_or(location);
            if !location.starts_with(self.root) {
                continue;
            }
            let path = location.join(".project");
            let lines = self.container_lines(&location, false);
            if lines.is_empty()
                || taken.contains(&path)
                || sources.iter().any(|(_, known, _)| known == &path)
            {
                continue;
            }
            let id = format!("eclipse-{}", project["name"].as_str().unwrap_or_default());
            sources.push((
                id,
                path,
                vec![read_only(
                    zh,
                    "containers",
                    text(zh, "Containers and variables", "容器与变量"),
                    text(zh,
                        "Eclipse WTP and MyEclipse containers resolved by Lithe inside JDT from project files, without changing .classpath: Web App Libraries is WEB-INF/lib and the utility modules deployed there, EAR Libraries is the library folder of each EAR that bundles the module, Server Runtime and Java EE libraries come from the servlet container. TOMCAT_HOME and CATALINA_HOME point at the servlet container.",
                        "Eclipse WTP 与 MyEclipse 的容器由 Lithe 在 JDT 内根据项目文件解析，不修改 .classpath：Web App Libraries 是 WEB-INF/lib 及部署到其中的工具模块；EAR Libraries 是引用此模块的 EAR 的库目录；Server Runtime 与 Java EE 库来自 Servlet 容器。TOMCAT_HOME、CATALINA_HOME 指向 Servlet 容器。"),
                    json!(lines),
                    "stringList",
                )],
            ));
        }
        sources
    }

    fn described_projects(&self) -> Vec<&Value> {
        self.described
            .and_then(|value| value["projects"].as_array())
            .map(|projects| projects.iter().collect())
            .unwrap_or_default()
    }

    /// One line per container, variable and missing library of the project
    /// at `location`. `brief` keeps only the containers this plugin resolves.
    fn container_lines(&self, location: &Path, brief: bool) -> Vec<String> {
        let zh = self.zh;
        let Some(project) = self.described_projects().into_iter().find(|project| {
            project["location"].as_str().is_some_and(|value| {
                let path = PathBuf::from(value);
                dunce::canonicalize(&path).unwrap_or(path) == location
            })
        }) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for entry in project["entries"].as_array().into_iter().flatten() {
            let kind = entry["containerKind"].as_str().unwrap_or("other");
            let status = entry["status"].as_str().unwrap_or("unbound");
            let ours = entry["kind"] == "container" && kind != "other";
            if brief && !ours {
                continue;
            }
            if entry["kind"] == "container" && kind == "other" && status != "unbound" {
                continue;
            }
            let name = entry["description"]
                .as_str()
                .or_else(|| entry["path"].as_str())
                .unwrap_or_default();
            let libraries = entry["libraries"].as_u64().unwrap_or(0);
            let projects = entry["projects"].as_u64().unwrap_or(0);
            let state = match status {
                "resolved" if entry["kind"] == "variable" => entry["resolvedPath"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                "resolved" if projects > 0 => {
                    if zh {
                        format!("{libraries} 个 JAR，{projects} 个项目")
                    } else {
                        format!(
                            "{}, {}",
                            count(libraries, "JAR"),
                            count(projects, "project")
                        )
                    }
                }
                "resolved" => {
                    if zh {
                        format!("{libraries} 个 JAR")
                    } else {
                        count(libraries, "JAR")
                    }
                }
                "empty" if kind == "serverRuntime" || kind == "javaEE" => {
                    text(zh, "no servlet container found", "未找到 Servlet 容器")
                }
                "empty" => text(zh, "empty", "为空"),
                "missing" => text(zh, "file does not exist", "文件不存在"),
                "disabled" => text(zh, "detection is off", "自动识别已关闭"),
                _ if entry["path"].as_str().is_some_and(|path| {
                    path.starts_with(crate::project_model::eclipse::USER_LIBRARY)
                }) =>
                {
                    text(
                        zh,
                        "user library defined in an Eclipse workspace; add its JARs to .classpath",
                        "用户库定义在 Eclipse 工作区，请把其 JAR 加入 .classpath",
                    )
                }
                _ => text(zh, "cannot be resolved", "无法解析"),
            };
            lines.push(
                format!(
                    "{name}{}{state}",
                    if brief {
                        if zh {
                            "（"
                        } else {
                            " ("
                        }
                    } else {
                        " — "
                    }
                ) + if brief {
                    if zh {
                        "）"
                    } else {
                        ")"
                    }
                } else {
                    ""
                },
            );
            if lines.len() >= MAX_LINES {
                break;
            }
        }
        lines
    }

    fn model_title(&self) -> String {
        let zh = self.zh;
        let model = self.model;
        let mut title = match model.kind {
            ModelKind::Maven => text(
                zh,
                "Maven project (imported by Maven)",
                "Maven 项目（由 Maven 导入）",
            ),
            ModelKind::Gradle => text(
                zh,
                "Gradle project (imported by Gradle)",
                "Gradle 项目（由 Gradle 导入）",
            ),
            ModelKind::Eclipse if zh => {
                format!("Eclipse 项目（{} 个 .project）", model.eclipse_projects)
            }
            ModelKind::Eclipse => format!(
                "Eclipse project ({} .project files)",
                model.eclipse_projects
            ),
            ModelKind::Idea if zh => format!(
                "IntelliJ IDEA 项目（{} 个模块，只读导入）",
                model.idea_modules
            ),
            ModelKind::Idea => format!(
                "IntelliJ IDEA project ({} modules, imported read-only)",
                model.idea_modules
            ),
            ModelKind::LegacyWeb => text(
                zh,
                "Legacy web project (no build file)",
                "传统 Web 项目（无构建文件）",
            ),
            ModelKind::Plain => text(zh, "Plain Java folder", "普通 Java 文件夹"),
        };
        if !model.enabled {
            title.push_str(if zh {
                " · 自动识别已关闭"
            } else {
                " · detection is off"
            });
        }
        if model.truncated {
            title.push_str(if zh {
                " · 文件过多，只识别了前面的部分"
            } else {
                " · too many files, only the first part was read"
            });
        }
        title
    }

    fn detected_lines(&self) -> Vec<String> {
        let zh = self.zh;
        if self.model.kind.managed() || !self.model.enabled {
            return Vec::new();
        }
        let mut lines: Vec<String> = self
            .model
            .libraries
            .iter()
            .take(MAX_LINES)
            .map(|item| {
                let origin = match item.origin {
                    Origin::WebLibraries => text(zh, "Web app libraries", "Web 应用库"),
                    Origin::EarLibraries => text(zh, "EAR libraries", "EAR 库"),
                    Origin::ManifestClassPath => "MANIFEST Class-Path".to_string(),
                    Origin::IdeaModuleLibrary => text(zh, "IDEA module library", "IDEA 模块库"),
                    Origin::IdeaProjectLibrary => text(zh, "IDEA project library", "IDEA 项目库"),
                    Origin::ServletContainer => text(zh, "Servlet container", "Servlet 容器"),
                };
                if zh {
                    format!(
                        "{} — {origin}（{}，{} 个 JAR）",
                        item.pattern, item.label, item.count
                    )
                } else {
                    format!(
                        "{} — {origin} ({}, {})",
                        item.pattern,
                        item.label,
                        count(item.count as u64, "JAR")
                    )
                }
            })
            .collect();
        if let Some(runtime) = &self.model.servlet.runtime {
            if lines.len() < MAX_LINES {
                lines.push(if zh {
                    format!(
                        "Servlet 容器 — {}（{} 个 JAR）",
                        runtime.name,
                        runtime.libraries.len()
                    )
                } else {
                    format!(
                        "Servlet container — {} ({})",
                        runtime.name,
                        count(runtime.libraries.len() as u64, "JAR")
                    )
                });
            }
        }
        lines
    }

    fn servlet_text(&self) -> String {
        let zh = self.zh;
        match &self.model.servlet.runtime {
            // Name and origin first, the folder last: a long folder is
            // what the field cuts off.
            Some(runtime) => {
                let (name, home) = (&runtime.name, runtime.home.display());
                match (&runtime.source, zh) {
                    (ServletSource::Setting, true) => format!("{name}（Java 设置）— {home}"),
                    (ServletSource::Setting, false) => format!("{name} (Java settings) — {home}"),
                    (ServletSource::Environment(variable), true) => format!("自动：{name}（来自 {variable}）— {home}"),
                    (ServletSource::Environment(variable), false) => format!("Automatic: {name} (from {variable}) — {home}"),
                    (ServletSource::Installed, true) => format!("自动：{name} — {home}"),
                    (ServletSource::Installed, false) => format!("Automatic: {name} — {home}"),
                }
            }
            None if self.model.servlet.setting_invalid => text(
                zh,
                "The Servlet container folder in the Java settings has no servlet API library (lib/servlet-api.jar); choose the folder of a Tomcat or Jetty installation.",
                "Java 设置中的“Servlet 容器文件夹”没有 Servlet API 库（lib/servlet-api.jar），请选择 Tomcat 或 Jetty 的安装文件夹。",
            ),
            None => text(
                zh,
                "Automatic (not found): choose a Tomcat or Jetty folder as Servlet container folder in the Java settings, or set CATALINA_HOME.",
                "自动（未找到）：在 Java 设置的“Servlet 容器文件夹”中选择 Tomcat 或 Jetty 的安装文件夹，或设置 CATALINA_HOME。",
            ),
        }
    }

    fn unresolved_lines(&self) -> Vec<String> {
        let zh = self.zh;
        self.model
            .unresolved
            .iter()
            .take(MAX_LINES)
            .map(|item| {
                let reason = match item.kind {
                    UnresolvedKind::MissingFile => text(zh, "file does not exist", "文件不存在"),
                    UnresolvedKind::MissingLibrary => {
                        text(zh, "project library is not defined", "项目库未定义")
                    }
                    UnresolvedKind::IdeLibrary => text(
                        zh,
                        "IDEA global or application library, kept outside the project",
                        "IDEA 全局库或应用库，不在项目内",
                    ),
                    UnresolvedKind::ClassFolder => text(
                        zh,
                        "class folder; JDT accepts only JAR files",
                        "类文件夹，JDT 只接受 JAR",
                    ),
                    UnresolvedKind::UnknownMacro => text(
                        zh,
                        "path variable known only to IDEA",
                        "路径变量只有 IDEA 知道",
                    ),
                    UnresolvedKind::OutsideWorkspace => text(
                        zh,
                        "source folder outside the workspace",
                        "源码文件夹在工作区之外",
                    ),
                    UnresolvedKind::UserLibrary => text(
                        zh,
                        "Eclipse user library, defined in an Eclipse workspace",
                        "Eclipse 用户库，定义在 Eclipse 工作区",
                    ),
                };
                format!("{} — {reason}", item.name)
            })
            .collect()
    }
}

/// `n` English things, singular for one.
fn count(n: u64, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// A read-only field whose value is also its effective value. List items
/// and text are cut to the contract's bounds.
fn read_only(
    zh: bool,
    id: &str,
    title: String,
    description: String,
    value: Value,
    kind: &str,
) -> Value {
    let value = match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(MAX_LINES)
                .filter_map(|item| {
                    item.as_str()
                        .map(|line| json!(truncate(line.to_string(), MAX_ITEM_BYTES)))
                })
                .collect(),
        ),
        Value::String(line) => json!(truncate(line, MAX_ITEM_BYTES)),
        other => other,
    };
    json!({
        "id": id,
        "title": title,
        "description": description,
        "kind": kind,
        "value": value,
        "effectiveValue": value,
        "effectiveSource": text(zh, "Detected from project files", "根据项目文件识别"),
    })
}

/// Absolute JDT paths as workspace-relative ones where possible.
fn relative_list(root: &Path, value: &Value) -> Value {
    match value.as_array() {
        Some(items) => json!(items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| {
                let path = PathBuf::from(item);
                let path = dunce::canonicalize(&path).unwrap_or(path);
                crate::project_model::relative_or_absolute(root, &path)
            })
            .collect::<Vec<_>>()),
        None => Value::Null,
    }
}

fn truncate(mut text: String, bytes: usize) -> String {
    if text.len() > bytes {
        let mut end = bytes.saturating_sub(3);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

/// The new include list and source folders after saving
/// [`STORE_DETECTED_FIELD`] as `store`, from the file's current `explicit`
/// values. `None` leaves a key as it is.
pub fn stored_entries(page: &Page<'_>, store: bool) -> (Option<Value>, Option<Option<Value>>) {
    let storable = page.storable();
    let mut include = page.explicit.include();
    let had = page.explicit.libraries.is_some();
    if store {
        if !had {
            include.push(UPSTREAM_DEFAULT_LIBRARIES.to_string());
        }
        for pattern in &storable {
            if !include.contains(pattern) {
                include.push(pattern.clone());
            }
        }
    } else {
        include.retain(|pattern| !storable.contains(pattern));
    }
    let libraries = (store || had).then(|| json!(include));
    let sources = page.storable_source_paths();
    let source_paths = if sources.is_empty() {
        None
    } else if store && page.explicit.source_paths.is_none() {
        Some(Some(json!(sources)))
    } else if !store && page.explicit.source_paths.as_ref() == Some(&json!(sources)) {
        Some(None)
    } else {
        None
    };
    (libraries, source_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::servlet::{ServletResolution, ServletRuntime};
    use crate::project_model::{Detected, Unresolved};

    fn model(kind: ModelKind) -> ProjectModel {
        ProjectModel {
            kind,
            enabled: true,
            descriptor: Some(".idea/modules.xml".into()),
            idea_modules: 2,
            eclipse_projects: 0,
            libraries: vec![Detected {
                pattern: "web/WEB-INF/lib/*.jar".into(),
                origin: Origin::WebLibraries,
                label: "web".into(),
                count: 3,
            }],
            source_paths: vec!["src".into(), "test".into()],
            unresolved: vec![Unresolved {
                kind: UnresolvedKind::IdeLibrary,
                name: "Global".into(),
            }],
            needs_servlet: true,
            servlet: ServletResolution {
                runtime: Some(ServletRuntime {
                    home: PathBuf::from("/opt/tomcat"),
                    libraries: vec![PathBuf::from("/opt/tomcat/lib/servlet-api.jar")],
                    name: "Apache Tomcat 9.0.98".into(),
                    version: Some("9.0.98".into()),
                    source: ServletSource::Installed,
                }),
                setting_invalid: false,
            },
            truncated: false,
        }
    }

    #[test]
    fn saving_detected_entries_keeps_the_upstream_default_and_removing_them_restores_the_file() {
        let model = model(ModelKind::Idea);
        let explicit = Explicit::default();
        let page = Page {
            zh: false,
            root: Path::new("/w"),
            model: &model,
            explicit: &explicit,
            described: None,
        };
        let (libraries, sources) = stored_entries(&page, true);
        assert_eq!(libraries, Some(json!(["lib/**", "web/WEB-INF/lib/*.jar"])));
        assert_eq!(sources, Some(Some(json!(["src", "test"]))));
        let explicit = Explicit::from_document(&json!({
            "java.project.referencedLibraries": ["lib/**", "web/WEB-INF/lib/*.jar", "own.jar"],
            "java.project.sourcePaths": ["src", "test"],
        }));
        let page = Page {
            zh: false,
            root: Path::new("/w"),
            model: &model,
            explicit: &explicit,
            described: None,
        };
        assert!(page.stored());
        let (libraries, sources) = stored_entries(&page, false);
        assert_eq!(libraries, Some(json!(["lib/**", "own.jar"])));
        assert_eq!(sources, Some(None));
    }

    #[test]
    fn the_model_rows_show_detection_servlet_container_and_unresolved_entries() {
        let model = model(ModelKind::Idea);
        let explicit = Explicit::default();
        let page = Page {
            zh: true,
            root: Path::new("/w"),
            model: &model,
            explicit: &explicit,
            described: None,
        };
        let sources = page.model_sources(&[]);
        assert_eq!(sources.len(), 1);
        let (_, path, fields) = &sources[0];
        assert_eq!(path, &PathBuf::from("/w/.idea/modules.xml"));
        let ids: Vec<_> = fields
            .iter()
            .map(|field| field["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "model",
                "detected",
                "source-folders",
                "servlet",
                "unresolved"
            ]
        );
        assert_eq!(
            fields[0]["value"],
            "IntelliJ IDEA 项目（2 个模块，只读导入）"
        );
        assert_eq!(
            fields[3]["value"],
            "自动：Apache Tomcat 9.0.98 — /opt/tomcat"
        );
        assert_eq!(
            fields[4]["value"],
            json!(["Global — IDEA 全局库或应用库，不在项目内"])
        );
        assert!(page
            .model_sources(&[PathBuf::from("/w/.idea/modules.xml")])
            .is_empty());
    }

    #[test]
    fn eclipse_rows_and_labels_come_from_what_jdt_resolved() {
        let model = ProjectModel {
            kind: ModelKind::Eclipse,
            ..model(ModelKind::Eclipse)
        };
        let explicit = Explicit::default();
        let root = std::env::temp_dir();
        let root = dunce::canonicalize(&root).unwrap();
        let described = json!({"projects": [{"name": "app", "location": root.join("app").to_str().unwrap(), "entries": [
            {"kind": "container", "containerKind": "other", "path": "org.eclipse.jdt.launching.JRE_CONTAINER", "status": "resolved", "libraries": 1},
            {"kind": "container", "containerKind": "webApp", "path": "org.eclipse.jst.j2ee.internal.web.container", "description": "Web App Libraries", "status": "resolved", "libraries": 37, "projects": 0},
            {"kind": "container", "containerKind": "serverRuntime", "path": "org.eclipse.jst.server.core.container/t/Tomcat", "description": "Server Runtime [Tomcat]", "status": "empty"},
            {"kind": "container", "containerKind": "other", "path": "org.eclipse.jdt.USER_LIBRARY/Libs", "status": "unbound"},
            {"kind": "variable", "path": "TOMCAT_HOME/lib/servlet-api.jar", "status": "resolved", "resolvedPath": "/opt/tomcat/lib/servlet-api.jar"},
            {"kind": "library", "path": "D:/old/x.jar", "status": "missing"}
        ]}]});
        let page = Page {
            zh: false,
            root: &root,
            model: &model,
            explicit: &explicit,
            described: Some(&described),
        };
        assert_eq!(
            page.classpath_label(&root.join("app")),
            "Eclipse JDT project settings; containers: Web App Libraries (37 JARs), Server Runtime [Tomcat] (no servlet container found)"
        );
        let rows = page.eclipse_sources(&[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, root.join("app/.project"));
        assert_eq!(
            rows[0].2[0]["value"],
            json!([
                "Web App Libraries — 37 JARs",
                "Server Runtime [Tomcat] — no servlet container found",
                "org.eclipse.jdt.USER_LIBRARY/Libs — user library defined in an Eclipse workspace; add its JARs to .classpath",
                "TOMCAT_HOME/lib/servlet-api.jar — /opt/tomcat/lib/servlet-api.jar",
                "D:/old/x.jar — file does not exist"
            ])
        );
    }
}
