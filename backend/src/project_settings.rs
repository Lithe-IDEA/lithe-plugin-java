//! Native compiler preferences and classpaths for unmanaged/Eclipse projects.
//! JDT owns project discovery and effective values. This module edits only
//! allowlisted native keys and stages replacements; the host owns publication.

use java_properties::{LineContent, PropertiesIter, PropertiesWriter};
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use lithe_plugin_sdk::{Context, FunctionError, PluginError, Value};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use url::Url;

use crate::project_model::{self, Environment, Explicit, ModelOptions, ProjectModel};

const LIMIT: u64 = 2 * 1024 * 1024;
const TARGET: &str = "org.eclipse.jdt.core.compiler.codegen.targetPlatform";
const SOURCE: &str = "org.eclipse.jdt.core.compiler.source";
const COMPLIANCE: &str = "org.eclipse.jdt.core.compiler.compliance";
const RELEASE: &str = "org.eclipse.jdt.core.compiler.release";
const NATURES: &str = "org.eclipse.jdt.ls.core.natureIds";
pub(crate) const LIBRARIES_FACT: &str = "org.eclipse.jdt.ls.core.referencedLibraries";
const PREFS: &str = ".settings/org.eclipse.jdt.core.prefs";
const SETTINGS: &str = ".vscode/settings.json";
const LIBRARY_KEY: &str = "java.project.referencedLibraries";
const SOURCE_PATHS_KEY: &str = "java.project.sourcePaths";

/// Bounded native read bindings; no configuration documents or tool models are cached.
#[derive(Default)]
pub struct ProjectSettings {
    bindings: Mutex<BTreeMap<String, BTreeMap<String, PathBuf>>>,
}
fn fail() -> FunctionError {
    PluginError::new(
        "java.nativeSettings.failed",
        "Reload Java project settings and check that their native files are readable.",
    )
    .into()
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn read(path: &Path) -> Result<(Vec<u8>, String), FunctionError> {
    let info = match fs::symlink_metadata(path) {
        Ok(info) => info,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), "missing".into()))
        }
        Err(_) => return Err(fail()),
    };
    if !info.is_file() || info.len() > LIMIT {
        return Err(fail());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| fail())?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail())?;
    if bytes.len() as u64 > LIMIT {
        return Err(fail());
    }
    let revision = digest(&bytes);
    Ok((bytes, revision))
}
fn root(context: &Context) -> Result<PathBuf, FunctionError> {
    let workspace = context.workspace_context().ok_or_else(fail)?;
    let response = context
        .host()
        .workspace()
        .get_context(workspace)
        .map_err(|_| fail())?;
    let path = response["folders"][0]["path"].as_str().ok_or_else(fail)?;
    dunce::canonicalize(path).map_err(|_| fail())
}
fn command(context: &Context, name: &str, args: Value) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let workspace = context.workspace_context().ok_or_else(fail)?;
    let answer=context.host().request(crate::settings_session::REQUEST_METHOD,json!({"context":workspace.to_value(),"serverID":"java","method":"workspace/executeCommand","params":{"command":name,"arguments":args},"timeoutMilliseconds":30000})).map_err(|_|fail())?;
    Ok(answer.get("value").cloned().unwrap_or(Value::Null))
}
fn local(zh: bool, en: &str, cn: &str) -> String {
    if zh { cn } else { en }.to_owned()
}
fn normalize(value: &str) -> String {
    value.strip_prefix("1.").unwrap_or(value).to_owned()
}
fn properties(bytes: &[u8]) -> Result<BTreeMap<String, String>, FunctionError> {
    java_properties::read(bytes)
        .map(|map| map.into_iter().collect())
        .map_err(|_| fail())
}
fn target(raw: &BTreeMap<String, String>) -> Value {
    raw.get(TARGET)
        .map(|v| json!(normalize(v)))
        .unwrap_or(Value::Null)
}
fn field(
    zh: bool,
    descriptor: (&str, &str, &str, &str),
    raw: Value,
    effective: Value,
    mut options: Value,
    help: &str,
) -> Value {
    let (id, en, cn, kind) = descriptor;
    if kind == "enum" {
        for value in [&raw, &effective] {
            if let Some(value) = value.as_str() {
                if !options
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|option| option["value"] == value)
                {
                    options
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"value":value,"title":value}));
                }
            }
        }
    }
    json!({"id":id,"title":local(zh,en,cn),"description":help,"kind":kind,"value":raw,"effectiveValue":effective,"effectiveSource":local(zh,"Eclipse JDT project settings","Eclipse JDT 项目生效配置"),"options":options})
}
fn compiler_fields(
    zh: bool,
    raw: &BTreeMap<String, String>,
    effective: &Value,
    module: bool,
) -> Vec<Value> {
    let options: Vec<_> = (1..=26)
        .map(|version| json!({"value":version.to_string(),"title":version.to_string()}))
        .collect();
    let help=local(zh,"Eclipse JDT compiles unmanaged/Eclipse projects. Maven and Gradle use their own compiler pages. The project JDK is selected in Toolchains.","普通 Java 与 Eclipse 项目使用 Eclipse JDT 编译；Maven、Gradle 使用各自编译器页面。项目 JDK 在工具链中选择。");
    let mut fields = vec![field(
        zh,
        ("target", "Target bytecode", "目标字节码", "enum"),
        target(raw),
        effective[TARGET]
            .as_str()
            .map(|v| json!(normalize(v)))
            .unwrap_or(Value::Null),
        json!(options),
        &help,
    )];
    if !module {
        fields.push(field(zh, ("release", "Use --release API restrictions", "使用 --release API 限制", "boolean"),raw.get(RELEASE).map(|v|json!(v=="enabled")).unwrap_or(Value::Null),effective[RELEASE].as_str().map(|v|json!(v=="enabled")).unwrap_or(Value::Null),json!([]),&local(zh,"For Java 9 and newer, restrict available JDK APIs as well as the generated class version.","Java 9 及更新版本同时限制可用 JDK API 和生成的 class 版本。")));
        let key = "org.eclipse.jdt.core.compiler.debug.lineNumber";
        fields.push(field(
            zh,
            (
                "debug",
                "Generate debugging information",
                "生成调试信息",
                "boolean",
            ),
            raw.get(key)
                .map(|v| json!(v == "generate"))
                .unwrap_or(Value::Null),
            effective[key]
                .as_str()
                .map(|v| json!(v == "generate"))
                .unwrap_or(Value::Null),
            json!([]),
            "",
        ));
    }
    fields
}

impl ProjectSettings {
    /// Projects native settings only after the declared JDT server is ready.
    pub fn snapshot(
        &self,
        params: Value,
        context: &Context,
        libraries: bool,
        session: &crate::settings_session::SettingsSession,
    ) -> Result<Value, FunctionError> {
        let root = root(context)?;
        let zh = params["context"]["language"]
            .as_str()
            .unwrap_or("")
            .starts_with("zh");
        if params["operation"] == "readAfterSave" {
            session
                .refresh_native_settings(context.workspace_context().ok_or_else(fail)?, &root)
                .map_err(|_| fail())?;
        }
        let projects = command(context, "java.project.getAll", json!([]))?;
        if params["operation"] == "readAfterSave" {
            let identifiers: Vec<_> = projects
                .as_array()
                .ok_or_else(fail)?
                .iter()
                .map(|uri| json!({"uri":uri}))
                .collect();
            context.host().request(crate::settings_session::NOTIFY_METHOD,json!({"context":context.workspace_context().ok_or_else(fail)?.to_value(),"serverID":"java","method":"java/projectConfigurationsUpdate","params":{"identifiers":identifiers,"force":true}})).map_err(|_|fail())?;
        }
        let keys = json!([
            TARGET,
            SOURCE,
            COMPLIANCE,
            RELEASE,
            "org.eclipse.jdt.core.compiler.debug.lineNumber",
            NATURES,
            LIBRARIES_FACT,
            crate::library_settings::SOURCE_PATHS_FACT
        ]);
        let mut facts = Vec::new();
        // Maven and Gradle projects: shown read-only on the structure page.
        let mut managed_projects = Vec::new();
        for entry in projects.as_array().ok_or_else(fail)? {
            context.check_cancelled()?;
            let value = entry.as_str().ok_or_else(fail)?;
            let path = Url::parse(value)
                .ok()
                .and_then(|u| u.to_file_path().ok())
                .and_then(|p| dunce::canonicalize(p).ok())
                .ok_or_else(fail)?;
            if !path.starts_with(&root) {
                continue;
            }
            let settings = command(context, "java.project.getSettings", json!([value, keys]))?;
            let managed = settings[NATURES].as_array().is_some_and(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|id| id.contains("m2e") || id.contains("buildship"))
            });
            if !managed {
                facts.push((path, settings));
            } else if libraries {
                managed_projects.push((path, value.to_owned()));
            }
        }
        if facts.len() + managed_projects.len() > 1023 {
            return Err(fail());
        }
        // Global defaults come from the native global preference file, or
        // from a project with no project-specific override. A module override
        // is never presented as the workspace default.
        let global_properties = properties(&read(&root.join(PREFS))?.0)?;
        let mut global_effective = json!({});
        let mut private_bytes = 0usize;
        let mut project_preferences = Vec::new();
        for (path, _) in &facts {
            let (bytes, _) = read(&path.join(PREFS))?;
            private_bytes += bytes.len();
            if private_bytes > 8 * 1024 * 1024 {
                return Err(fail());
            }
            project_preferences.push(properties(&bytes)?);
        }
        for key in [
            TARGET,
            RELEASE,
            "org.eclipse.jdt.core.compiler.debug.lineNumber",
        ] {
            if let Some(value) = global_properties.get(key) {
                global_effective[key] = json!(value);
            } else if let Some((index, _)) = project_preferences
                .iter()
                .enumerate()
                .find(|(_, values)| !values.contains_key(key))
            {
                global_effective[key] = facts[index].1[key].clone();
            }
        }
        // The libraries page also projects the detected project model and
        // what the project model bundle made of Eclipse containers. A JDT LS
        // without the bundle (an older user installation) has no describe
        // command; the page then shows only what the backend detected.
        let model = if libraries {
            let workspace = context.workspace_context().ok_or_else(fail)?;
            let options = session.model_options(workspace.context_id());
            let explicit = Explicit::from_document(&native_json(&read(&root.join(SETTINGS))?.0)?);
            let model = project_model::detect(&root, &options, &Environment::current());
            let described = command(
                context,
                crate::library_settings::DESCRIBE_COMMAND,
                json!([]),
            )
            .ok();
            Some((model, explicit, described))
        } else {
            None
        };
        let page =
            model.as_ref().map(
                |(model, explicit, described)| crate::library_settings::Page {
                    zh,
                    root: &root,
                    model,
                    explicit,
                    described: described.as_ref(),
                },
            );
        let mut sources = Vec::new();
        let mut bindings = BTreeMap::new();
        let mut paths = BTreeSet::new();
        if !libraries {
            let path = root.join(PREFS);
            let (bytes, revision) = read(&path)?;
            let raw = properties(&bytes)?;
            let effective = global_effective.clone();
            let id = "defaults";
            bindings.insert(id.into(), path.clone());
            paths.insert(path);
            sources.push(json!({"id":id,"group":"project","title":local(zh,"Project defaults","项目默认设置"),"path":PREFS,"revision":revision,"writable":true,"fields":compiler_fields(zh,&raw,&effective,false)}));
        }
        let mut modules = Vec::new();
        for (path, fact) in facts {
            if libraries && read(&path.join(".classpath"))?.1 != "missing" {
                modules.push(path);
                continue;
            }
            let source_path = if libraries {
                root.join(SETTINGS)
            } else {
                path.join(PREFS)
            };
            if !paths.insert(source_path.clone()) {
                continue;
            }
            let rel = source_path
                .strip_prefix(&root)
                .map_err(|_| fail())?
                .to_str()
                .ok_or_else(fail)?
                .replace('\\', "/");
            let id = format!("source{:x}", Sha256::digest(rel.as_bytes()));
            let (bytes, revision) = read(&source_path)?;
            let mut fields = if let Some(page) = &page {
                page.settings_fields(&fact, library_input(&native_json(&bytes)?))
            } else {
                compiler_fields(zh, &properties(&bytes)?, &fact, true)
            };
            if !libraries {
                for field in &mut fields {
                    if field["id"] == "target" {
                        field["inheritedValue"] = global_effective[TARGET]
                            .as_str()
                            .map(|value| json!(normalize(value)))
                            .unwrap_or(Value::Null);
                    }
                }
            }
            bindings.insert(id.clone(), source_path);
            sources.push(json!({"id":id,"group":if libraries{"libraries"}else{"modules"},"title":path.file_name().and_then(|n|n.to_str()).unwrap_or("Java"),"path":rel,"revision":revision,"writable":true,"fields":fields}));
        }
        let module_sources = if libraries {
            self.module_sources(
                context,
                zh,
                &root,
                &modules,
                &managed_projects,
                &mut bindings,
                &mut paths,
            )?
        } else {
            Vec::new()
        };
        let mut groups = if libraries {
            let mut groups = Vec::new();
            if module_sources
                .iter()
                .any(|source| source["group"] == "modules")
            {
                groups.push(
                    json!({"id":"modules","title":local(zh,"Modules","模块"),"layout":"sidebar"}),
                );
            }
            if module_sources
                .iter()
                .any(|source| source["group"] == "facets")
            {
                groups.push(json!({"id":"facets","title":local(zh,"Web Facets","Web Facet"),"layout":"sidebar"}));
            }
            if module_sources.is_empty()
                || sources.iter().any(|source| source["group"] == "libraries")
            {
                groups.push(json!({"id":"libraries","title":local(zh,"Referenced libraries","引用的库"),"layout":"form"}));
            }
            groups
        } else {
            vec![
                json!({"id":"project","title":local(zh,"Project defaults","项目默认设置"),"layout":"form"}),
                json!({"id":"modules","title":local(zh,"Per-module bytecode","按模块设置字节码"),"rowTitle":local(zh,"Module","模块"),"layout":"table"}),
            ]
        };
        sources.splice(0..0, module_sources);
        if let Some(page) = &page {
            let taken: Vec<PathBuf> = paths.iter().cloned().collect();
            let mut model_sources: Vec<(PathBuf, Value)> = Vec::new();
            let mut rows = page.model_sources(&taken);
            rows.extend(page.eclipse_sources(&taken));
            for (id, path, fields) in rows {
                // An Eclipse project's container rows share its `.project`
                // with the model rows; one source holds both.
                if let Some(source) = model_sources.iter_mut().find(|(known, _)| known == &path) {
                    let existing = source.1["fields"].as_array_mut().ok_or_else(fail)?;
                    existing.extend(
                        fields
                            .into_iter()
                            .take(32usize.saturating_sub(existing.len())),
                    );
                    continue;
                }
                if sources.len() + model_sources.len() >= 1024 || !paths.insert(path.clone()) {
                    continue;
                }
                let rel = project_model::relative_inside(&root, &path).ok_or_else(fail)?;
                let title = if id == "model" {
                    local(zh, "Project model", "工程模型")
                } else {
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("Eclipse")
                        .to_string()
                };
                let (_, revision) = read(&path)?;
                let id = format!("model{:x}", Sha256::digest(rel.as_bytes()));
                model_sources.push((path, json!({"id":id,"group":"model","title":title,"path":rel,"revision":revision,"writable":false,"fields":fields})));
            }
            sources.extend(model_sources.into_iter().map(|(_, source)| source));
            if sources.iter().any(|source| source["group"] == "model") {
                groups.push(json!({"id":"model","title":local(zh,"Project model","工程模型"),"layout":"form"}));
            }
        }
        let mut all = self.bindings.lock().map_err(|_| fail())?;
        if all.len() >= 32 && !all.contains_key(&root.to_string_lossy().to_string()) {
            return Err(fail());
        }
        all.insert(root.to_string_lossy().to_string(), bindings);
        Ok(
            json!({"format":"litheSettingsSources","version":0,"title":if libraries{local(zh,"Java Project Structure","Java 项目结构")}else{local(zh,"Java Compiler","Java 编译器")},"groups":groups,"sources":sources}),
        )
    }
    /// The modules of the structure page, parents before children: Eclipse
    /// projects edit their `.classpath`; Maven and Gradle projects show the
    /// classpath JDT resolved, read-only.
    #[allow(clippy::too_many_arguments)]
    fn module_sources(
        &self,
        context: &Context,
        zh: bool,
        root: &Path,
        eclipse: &[PathBuf],
        managed: &[(PathBuf, String)],
        bindings: &mut BTreeMap<String, PathBuf>,
        paths: &mut BTreeSet<PathBuf>,
    ) -> Result<Vec<Value>, FunctionError> {
        use crate::module_structure as modules;
        let mut classpaths = Vec::new();
        let mut containers = Vec::new();
        let mut names = Vec::new();
        for project in eclipse {
            let file = project.join(".classpath");
            let (bytes, revision) = read(&file)?;
            let classpath = modules::read(zh, &bytes)?;
            for container in &classpath.containers {
                if !containers.contains(container) {
                    containers.push(container.clone());
                }
            }
            let name = modules::project_name(project);
            names.push(name.clone());
            classpaths.push((project.clone(), file, revision, classpath, name));
        }
        // (folder, source file, source) in folder order, so that an
        // enclosing module comes before the modules inside it.
        let mut rows: Vec<(PathBuf, PathBuf, Value)> = Vec::new();
        let mut facet_sources = Vec::new();
        for (project, file, revision, classpath, name) in classpaths {
            let rel = project_model::relative_inside(root, &file).ok_or_else(fail)?;
            let choices = modules::Choices {
                containers: &containers,
                modules: &names,
            };
            let fields = modules::fields(zh, &classpath, &name, &choices, &rel);
            let id = format!("source{:x}", Sha256::digest(rel.as_bytes()));
            if classpath.editable {
                bindings.insert(id.clone(), file.clone());
            }
            // Web Facet is stored in another native XML source. Keep it
            // separate from .classpath so each save replaces exactly one file.
            if facet_sources.len() < 256 {
                let facet_file = project.join(crate::web_facet::FILE);
                if fs::symlink_metadata(&facet_file).is_ok_and(|info| info.is_file()) {
                    let (bytes, revision) = read(&facet_file)?;
                    if let Some(fields) = crate::web_facet::fields(zh, &bytes)? {
                        let rel =
                            project_model::relative_inside(root, &facet_file).ok_or_else(fail)?;
                        let facet_id = format!("facet{:x}", Sha256::digest(rel.as_bytes()));
                        if paths.insert(facet_file.clone()) {
                            bindings.insert(facet_id.clone(), facet_file);
                            facet_sources.push(json!({"id":facet_id,"group":"facets","title":name.clone(),"path":rel,"revision":revision,"writable":true,"fields":fields}));
                        }
                    }
                }
            }
            rows.push((project, file, json!({"id":id,"group":"modules","title":name,"path":rel,"revision":revision,"writable":classpath.editable,"fields":fields})));
        }
        let mut unavailable = 0;
        for (project, uri) in managed {
            context.check_cancelled()?;
            let Some(file) = ["pom.xml", "build.gradle.kts", "build.gradle"]
                .iter()
                .map(|name| project.join(name))
                .find(|file| file.is_file())
            else {
                continue;
            };
            // JDT reads the options from a JSON string, as vscode-java sends them.
            let resolved = command(
                context,
                "java.project.getClasspaths",
                json!([uri, r#"{"scope":"test"}"#]),
            );
            let Some(resolved) = resolved.ok().and_then(|value| {
                value["classpaths"].as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
            }) else {
                unavailable += 1;
                continue;
            };
            let rel = project_model::relative_inside(root, &file).ok_or_else(fail)?;
            let (_, revision) = read(&file)?;
            let title = project
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Java")
                .to_owned();
            let id = format!("managed{:x}", Sha256::digest(rel.as_bytes()));
            rows.push((project.clone(), file, json!({"id":id,"group":"modules","title":title,"path":rel,"revision":revision,"writable":false,"fields":modules::resolved_fields(zh,root,&resolved,&rel)})));
        }
        if unavailable > 0 {
            // A module JDT could not resolve is left out; the page still shows the others.
            let _ = context.diagnostics().degraded(
                context.function_id(),
                "java.moduleSettings.classpathUnavailable",
                Some(unavailable),
            );
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut depths: Vec<(PathBuf, String, usize)> = Vec::new();
        let mut sources = Vec::new();
        for (project, file, mut source) in rows {
            if !paths.insert(file) {
                continue;
            }
            // The nearest enclosing module is the parent, within the
            // contract's eight levels.
            if let Some((_, parent, depth)) = depths
                .iter()
                .filter(|(folder, _, depth)| {
                    project != *folder && project.starts_with(folder) && *depth < 7
                })
                .max_by_key(|(folder, _, _)| folder.components().count())
            {
                source["parent"] = json!(parent);
                depths.push((
                    project,
                    source["id"].as_str().unwrap_or("").to_owned(),
                    depth + 1,
                ));
            } else {
                depths.push((project, source["id"].as_str().unwrap_or("").to_owned(), 0));
            }
            sources.push(source);
        }
        sources.extend(facet_sources);
        Ok(sources)
    }
    /// Prepares only the confirmed native source; publication belongs to the host.
    pub fn edit(
        &self,
        params: Value,
        context: &Context,
        session: &crate::settings_session::SettingsSession,
    ) -> Result<Value, FunctionError> {
        let root = root(context)?;
        let id = params["sourceID"].as_str().ok_or_else(fail)?;
        let path = self
            .bindings
            .lock()
            .map_err(|_| fail())?
            .get(&root.to_string_lossy().to_string())
            .and_then(|b| b.get(id))
            .cloned()
            .ok_or_else(fail)?;
        let parent = path.parent().ok_or_else(fail)?;
        let mut existing = parent;
        while !existing.exists() {
            existing = existing.parent().ok_or_else(fail)?;
        }
        if !dunce::canonicalize(existing)
            .map_err(|_| fail())?
            .starts_with(&root)
        {
            return Err(fail());
        }
        let (bytes, revision) = read(&path)?;
        if params["revision"] != revision {
            return Err(fail());
        }
        let values = params["values"].as_object().ok_or_else(fail)?;
        let replacement = if path.ends_with(".classpath") {
            crate::module_structure::edit(&bytes, parent, values)?
        } else if path.ends_with(crate::web_facet::FILE) {
            crate::web_facet::edit(&bytes, values)?
        } else if path.ends_with(SETTINGS) {
            let workspace = context.workspace_context().ok_or_else(fail)?;
            let options = session.model_options(workspace.context_id());
            edit_settings(&root, &bytes, values, &options)?
        } else {
            edit_properties(&bytes, values)?
        };
        if replacement.len() as u64 > LIMIT {
            return Err(fail());
        }
        context.check_cancelled()?;
        let stage = params["outputFile"].as_str().ok_or_else(fail)?;
        let mut options = OpenOptions::new();
        options.write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let mut output = options.open(stage).map_err(|_| fail())?;
        if !output.metadata().map_err(|_| fail())?.is_file() {
            return Err(fail());
        }
        context.check_cancelled()?;
        output.write_all(&replacement).map_err(|_| fail())?;
        Ok(
            json!({"format":"litheSettingsSourceEdit","version":0,"sourceID":id,"revision":digest(&replacement),"changed":bytes!=replacement}),
        )
    }
}

/// Reads the actual upstream settings sources and overlays the detected
/// project model; used at startup and on every push. Returns the `java`
/// sections to overlay and the model they came from.
pub fn native_settings(
    root: &Path,
    options: &ModelOptions,
) -> Result<(Value, ProjectModel), FunctionError> {
    let mut settings = json!({});
    if read(&root.join(PREFS))?.1 != "missing" {
        settings["settings"] =
            json!({"url":Url::from_file_path(root.join(PREFS)).map_err(|_|fail())?.to_string()});
    }
    let (bytes, _) = read(&root.join(SETTINGS))?;
    let explicit = Explicit::from_document(&native_json(&bytes)?);
    let model = project_model::detect(root, options, &Environment::current());
    let overlay = project_model::overlay(&model, &explicit);
    for section in ["project", "lithe"] {
        if overlay
            .get(section)
            .is_some_and(|value| value != &json!({}))
        {
            settings[section] = overlay[section].clone();
        }
    }
    Ok((settings, model))
}
fn native_json(bytes: &[u8]) -> Result<Value, FunctionError> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| fail())?;
    let value: Option<Value> =
        jsonc_parser::parse_to_serde_value(text, &Default::default()).map_err(|_| fail())?;
    let value = value.unwrap_or(json!({}));
    if !value.is_object() {
        return Err(fail());
    }
    Ok(value)
}
fn library_input(settings: &Value) -> Value {
    match settings.get(LIBRARY_KEY) {
        Some(Value::Object(object)) => object.get("include").cloned().unwrap_or(Value::Null),
        Some(value) => value.clone(),
        None => Value::Null,
    }
}
/// Applies the confirmed fields of the `.vscode/settings.json` source in
/// order: libraries, source folders, then saving or removing the detected
/// entries against the file as the first two left it.
fn edit_settings(
    root: &Path,
    bytes: &[u8],
    values: &serde_json::Map<String, Value>,
    options: &ModelOptions,
) -> Result<Vec<u8>, FunctionError> {
    use crate::library_settings::{LIBRARIES_FIELD, SOURCE_PATHS_FIELD, STORE_DETECTED_FIELD};
    let mut document = bytes.to_vec();
    if let Some(libraries) = values.get(LIBRARIES_FIELD) {
        document = edit_json(&document, LIBRARY_KEY, libraries)?;
    }
    if let Some(paths) = values.get(SOURCE_PATHS_FIELD) {
        document = edit_json(&document, SOURCE_PATHS_KEY, paths)?;
    }
    if let Some(store) = values.get(STORE_DETECTED_FIELD) {
        let store = match store {
            Value::Bool(store) => *store,
            Value::Null => false,
            _ => return Err(fail()),
        };
        let explicit = Explicit::from_document(&native_json(&document)?);
        let model = project_model::detect(root, options, &Environment::current());
        let page = crate::library_settings::Page {
            zh: false,
            root,
            model: &model,
            explicit: &explicit,
            described: None,
        };
        let (libraries, source_paths) = crate::library_settings::stored_entries(&page, store);
        if let Some(libraries) = libraries {
            document = edit_json(&document, LIBRARY_KEY, &libraries)?;
        }
        if let Some(source_paths) = source_paths {
            document = edit_json(
                &document,
                SOURCE_PATHS_KEY,
                &source_paths.unwrap_or(Value::Null),
            )?;
        }
    }
    Ok(document)
}
/// Sets or (with null) removes the string list `key`; a rich
/// `referencedLibraries` object keeps its excludes and sources and only
/// its `include` changes.
fn edit_json(bytes: &[u8], key: &str, libraries: &Value) -> Result<Vec<u8>, FunctionError> {
    native_json(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(|_| fail())?;
    let root = CstRootNode::parse(text, &Default::default()).map_err(|_| fail())?;
    let object = root.object_value_or_create().ok_or_else(fail)?;
    let rich = (key == LIBRARY_KEY)
        .then(|| object.get(LIBRARY_KEY).and_then(|prop| prop.object_value()))
        .flatten();
    let (target, key) = rich.map(|rich| (rich, "include")).unwrap_or((object, key));
    if libraries.is_null() {
        if let Some(prop) = target.get(key) {
            prop.remove();
        }
    } else {
        let values = libraries
            .as_array()
            .ok_or_else(fail)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| CstInputValue::String(s.to_owned()))
                    .ok_or_else(fail)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let input = CstInputValue::Array(values);
        if let Some(prop) = target.get(key) {
            prop.set_value(input);
        } else {
            target.append(key, input);
        }
    }
    Ok(root.to_string().into_bytes())
}
/// Serializes an edited `.classpath` root in place of the original one,
/// keeping the nodes outside the root, including comments and processing
/// instructions. xmltree validates each node before this serialization.
pub(crate) fn write_classpath(
    bytes: &[u8],
    doc: &xmltree::Element,
) -> Result<Vec<u8>, FunctionError> {
    let nodes = xmltree::Element::parse_all(bytes).map_err(|_| fail())?;
    let mut out = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".to_vec();
    for node in nodes {
        match node {
            xmltree::XMLNode::Element(_) => doc
                .write_with_config(
                    &mut out,
                    xmltree::EmitterConfig::new()
                        .perform_indent(true)
                        .write_document_declaration(false),
                )
                .map_err(|_| fail())?,
            xmltree::XMLNode::Comment(text) => {
                write!(&mut out, "<!--{text}-->").map_err(|_| fail())?;
            }
            xmltree::XMLNode::ProcessingInstruction(name, value) => {
                write!(
                    &mut out,
                    "<?{name}{}?>",
                    value.map(|v| format!(" {v}")).unwrap_or_default()
                )
                .map_err(|_| fail())?;
            }
            xmltree::XMLNode::Text(text) => out.extend_from_slice(text.as_bytes()),
            xmltree::XMLNode::CData(_) => return Err(fail()),
        }
    }
    Ok(out)
}
fn edit_properties(
    bytes: &[u8],
    values: &serde_json::Map<String, Value>,
) -> Result<Vec<u8>, FunctionError> {
    properties(bytes)?;
    let mut edits = BTreeMap::<String, Option<String>>::new();
    for (id, value) in values {
        match id.as_str() {
            "target" => {
                let version = value.as_str();
                if version
                    .is_some_and(|v| v.parse::<u32>().map_or(true, |n| !(1..=26).contains(&n)))
                {
                    return Err(fail());
                }
                for key in [TARGET, SOURCE, COMPLIANCE] {
                    edits.insert(
                        key.into(),
                        version.map(|v| {
                            if v.parse::<u32>().is_ok_and(|n| n <= 8) {
                                format!("1.{v}")
                            } else {
                                v.into()
                            }
                        }),
                    );
                }
            }
            "release" => {
                edits.insert(
                    RELEASE.into(),
                    if value.is_null() {
                        None
                    } else {
                        Some(
                            if value.as_bool().ok_or_else(fail)? {
                                "enabled"
                            } else {
                                "disabled"
                            }
                            .into(),
                        )
                    },
                );
            }
            "debug" => {
                for key in ["lineNumber", "sourceFile", "localVariable"] {
                    edits.insert(
                        format!("org.eclipse.jdt.core.compiler.debug.{key}"),
                        if value.is_null() {
                            None
                        } else {
                            Some(
                                if value.as_bool().ok_or_else(fail)? {
                                    "generate"
                                } else {
                                    "do not generate"
                                }
                                .into(),
                            )
                        },
                    );
                }
            }
            _ => return Err(fail()),
        }
    }
    let lines: Vec<_> = PropertiesIter::new(bytes)
        .collect::<Result<_, _>>()
        .map_err(|_| fail())?;
    let mut offsets = vec![0];
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            index += 1;
            if bytes.get(index) == Some(&b'\n') {
                index += 1;
            }
            offsets.push(index);
        } else if bytes[index] == b'\n' {
            index += 1;
            offsets.push(index);
        } else {
            index += 1;
        }
    }
    let mut out = Vec::new();
    let mut cursor = 0;
    for (index, line) in lines.iter().enumerate() {
        if let LineContent::KVPair(key, _) = line.content() {
            if edits.contains_key(key) {
                let start = *offsets.get(line.line_number() - 1).ok_or_else(fail)?;
                let end = lines
                    .get(index + 1)
                    .and_then(|next| offsets.get(next.line_number() - 1))
                    .copied()
                    .unwrap_or(bytes.len());
                out.extend_from_slice(&bytes[cursor..start]);
                cursor = end;
            }
        }
    }
    out.extend_from_slice(&bytes[cursor..]);
    if !out.is_empty() && !out.ends_with(b"\n") && !out.ends_with(b"\r") {
        out.push(b'\n');
    }
    let mut writer = PropertiesWriter::new(&mut out);
    for (key, value) in edits {
        if let Some(value) = value {
            writer.write(&key, &value).map_err(|_| fail())?;
        }
    }
    writer.finish().map_err(|_| fail())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            for _ in 0..32 {
                let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("lithe-native-settings-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("temporary directory: {error}"),
                }
            }
            panic!("temporary directory collision limit");
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if !std::thread::panicking() {
                    panic!("temporary directory cleanup: {error}");
                }
            }
        }
    }
    #[test]
    fn rich_library_configuration_keeps_excludes_sources_and_comments() {
        let source = br#"{ // private comment
          "java.project.referencedLibraries": {
            "include": ["old.jar"],
            "exclude": ["secret/**/*.jar"],
            "sources": {"old.jar": "old-sources.jar"}
          }, "privateToken": "untouched"
        }"#;
        let updated = edit_json(source, LIBRARY_KEY, &json!(["new.jar"])).unwrap();
        let parsed = native_json(&updated).unwrap();
        assert_eq!(parsed[LIBRARY_KEY]["include"], json!(["new.jar"]));
        assert_eq!(parsed[LIBRARY_KEY]["exclude"], json!(["secret/**/*.jar"]));
        assert_eq!(parsed[LIBRARY_KEY]["sources"]["old.jar"], "old-sources.jar");
        assert!(String::from_utf8(updated.clone())
            .unwrap()
            .contains("private comment"));
        let reset = native_json(&edit_json(&updated, LIBRARY_KEY, &Value::Null).unwrap()).unwrap();
        assert!(reset[LIBRARY_KEY].get("include").is_none());
        assert_eq!(
            reset[LIBRARY_KEY]["exclude"],
            parsed[LIBRARY_KEY]["exclude"]
        );
        assert_eq!(reset["privateToken"], "untouched");
    }

    #[test]
    fn properties_edits_preserve_comments_private_keys_and_escaped_continuations() {
        let source=b"# keep this comment\r\neclipse.preferences.version=1\r\nprivateKey=private-value\r\norg.eclipse.jdt.core.compiler.codegen.targetPlatform : 1.8\r\norg.eclipse.jdt.core.compiler.sou\\\r\n rce=1.8\r\n! trailing comment\r\n";
        let updated = edit_properties(source, json!({"target":"11"}).as_object().unwrap()).unwrap();
        let text = String::from_utf8(updated.clone()).unwrap();
        assert!(text.contains("# keep this comment\r\n"));
        assert!(text.contains("! trailing comment"));
        assert!(text.contains("privateKey=private-value\r\n"));
        let values = properties(&updated).unwrap();
        assert_eq!(values[TARGET], "11");
        assert_eq!(values[SOURCE], "11");
        assert_eq!(values[COMPLIANCE], "11");
        let inherited =
            edit_properties(&updated, json!({"target":null}).as_object().unwrap()).unwrap();
        let values = properties(&inherited).unwrap();
        for key in [TARGET, SOURCE, COMPLIANCE] {
            assert!(!values.contains_key(key));
        }
        assert_eq!(values["privateKey"], "private-value");
    }
    #[test]
    fn jsonc_library_edits_preserve_comments_unknown_settings_and_spaces() {
        let source=b"{ // keep comment\n \"privateToken\": \"private-value\",\n \"java.project.referencedLibraries\": [\"old.jar\"],\n}";
        let updated = edit_json(
            source,
            LIBRARY_KEY,
            &json!(["C:\\Program Files\\Java\\external.jar"]),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&updated).contains("// keep comment"));
        let value = native_json(&updated).unwrap();
        assert_eq!(
            value[LIBRARY_KEY],
            json!(["C:\\Program Files\\Java\\external.jar"])
        );
        assert_eq!(value["privateToken"], "private-value");
        let reset = edit_json(&updated, LIBRARY_KEY, &Value::Null).unwrap();
        assert!(native_json(&reset).unwrap().get(LIBRARY_KEY).is_none());
    }
    #[test]
    fn complete_configuration_keeps_runtime_and_imports_when_native_keys_are_added() {
        let directory = TestDirectory::new();
        let root = &directory.0;
        fs::create_dir_all(root.join(".vscode")).unwrap();
        fs::write(
            root.join(SETTINGS),
            br#"{"java.project.referencedLibraries":["lib/**/*.jar"]}"#,
        )
        .unwrap();
        let original = json!({"java":{"configuration":{"runtimes":[{"name":"JavaSE-17","path":"/selected/jdk"}]},"import":{"maven":{"enabled":true}}}});
        let updated = crate::settings_session::with_native_settings(
            original.clone(),
            root,
            &ModelOptions::default(),
        )
        .unwrap();
        assert_eq!(
            updated["java"]["configuration"],
            original["java"]["configuration"]
        );
        assert_eq!(updated["java"]["import"], original["java"]["import"]);
        assert_eq!(
            updated["java"]["project"]["referencedLibraries"],
            json!(["lib/**/*.jar"])
        );
    }
}
