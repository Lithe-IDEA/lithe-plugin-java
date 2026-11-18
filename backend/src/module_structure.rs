//! Eclipse projects as modules of the Java project structure page, after
//! IntelliJ IDEA's Project Structure → Modules: each project's source folders,
//! output folder, JRE and dependencies, read from and written back to its
//! `.classpath`. Maven and Gradle modules show what JDT resolved, read-only,
//! because their build files own them.
//!
//! An edit rewrites only the entries the page manages. Entries are matched by
//! kind and path, so an unchanged entry keeps its other attributes (source
//! attachments, Javadoc locations, access rules); nodes the page does not show
//! stay where they are.

use lithe_plugin_sdk::{FunctionError, PluginError, Value};
use serde_json::{json, Map};
use std::path::Path;
use xmltree::{Element, XMLNode};

pub const ROOTS_FIELD: &str = "roots";
pub const OUTPUT_FIELD: &str = "output";
pub const JRE_FIELD: &str = "jre";
pub const DEPENDENCIES_FIELD: &str = "dependencies";

const JRE_CONTAINER: &str = "org.eclipse.jdt.launching.JRE_CONTAINER";
const STANDARD_VM: &str = "org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType";
const EXECUTION_ENVIRONMENTS: &[&str] = &[
    "JavaSE-1.8",
    "JavaSE-11",
    "JavaSE-17",
    "JavaSE-21",
    "JavaSE-25",
];
/// Containers offered to every module besides those the workspace uses.
const KNOWN_CONTAINERS: &[&str] = &[
    "org.eclipse.jdt.junit.JUNIT_CONTAINER/4",
    "org.eclipse.jdt.junit.JUNIT_CONTAINER/5",
    "org.eclipse.jst.j2ee.internal.web.container",
    "org.eclipse.jst.j2ee.internal.module.container",
];
/// The contract's bounds on one entry list and its values.
const MAX_ENTRIES: usize = 256;
const MAX_CHOICES: usize = 1024;
const MAX_VALUE: usize = 4096;

const FOLDER: &str = "folder";
const ARCHIVE: &str = "archive";
const CLASSES: &str = "classes";
const LIBRARY: &str = "library";
const MODULE: &str = "module";
const VARIABLE: &str = "variable";
const TEST: &str = "test";
const EXPORTED: &str = "exported";

fn fail() -> FunctionError {
    PluginError::new(
        "java.moduleSettings.failed",
        "Reload Java project settings and check that the module's .classpath is readable.",
    )
    .into()
}

fn outside() -> FunctionError {
    PluginError::new(
        "java.moduleSettings.sourceOutsideModule",
        "Choose a source folder inside the module's folder.",
    )
    .into()
}

fn local(zh: bool, en: &str, cn: &str) -> String {
    if zh { cn } else { en }.to_owned()
}

/// What a `.classpath` entry is on the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Root,
    Jre,
    Dependency(&'static str),
    Output,
    Other,
}

fn role(element: &Element) -> Role {
    if element.name != "classpathentry" {
        return Role::Other;
    }
    let path = attribute(element, "path");
    match attribute(element, "kind") {
        "src" if path.starts_with('/') => Role::Dependency(MODULE),
        "src" => Role::Root,
        "con" if path == JRE_CONTAINER || path.starts_with(&format!("{JRE_CONTAINER}/")) => {
            Role::Jre
        }
        "con" => Role::Dependency(LIBRARY),
        "lib" if archive(path) => Role::Dependency(ARCHIVE),
        "lib" => Role::Dependency(CLASSES),
        "var" => Role::Dependency(VARIABLE),
        "output" => Role::Output,
        _ => Role::Other,
    }
}

fn attribute<'a>(element: &'a Element, name: &str) -> &'a str {
    element
        .attributes
        .get(name)
        .map(String::as_str)
        .unwrap_or("")
}

fn archive(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".jar") || lower.ends_with(".zip")
}

/// The `.classpath` kind an entry kind of the page is written as.
fn native_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        FOLDER | MODULE => "src",
        ARCHIVE | CLASSES => "lib",
        LIBRARY => "con",
        VARIABLE => "var",
        _ => return None,
    })
}

/// The value of a nested `<attributes><attribute name value/></attributes>`.
fn nested(element: &Element, name: &str) -> Option<String> {
    element
        .get_child("attributes")?
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .find(|child| child.name == "attribute" && attribute(child, "name") == name)
        .map(|child| attribute(child, "value").to_owned())
}

fn set_nested(element: &mut Element, name: &str, value: Option<&str>) {
    if element.get_child("attributes").is_none() {
        if value.is_none() {
            return;
        }
        element
            .children
            .push(XMLNode::Element(Element::new("attributes")));
    }
    let Some(attributes) = element.get_mut_child("attributes") else {
        return;
    };
    let existing = attributes.children.iter().position(|node| {
        node.as_element()
            .is_some_and(|child| child.name == "attribute" && attribute(child, "name") == name)
    });
    match (existing, value) {
        (Some(index), Some(value)) => {
            if let Some(XMLNode::Element(child)) = attributes.children.get_mut(index) {
                child.attributes.insert("value".into(), value.into());
            }
        }
        (Some(index), None) => {
            attributes.children.remove(index);
        }
        (None, Some(value)) => {
            let mut child = Element::new("attribute");
            child.attributes.insert("name".into(), name.into());
            child.attributes.insert("value".into(), value.into());
            attributes.children.push(XMLNode::Element(child));
        }
        (None, None) => {}
    }
    let empty = element.get_child("attributes").is_some_and(|attributes| {
        !attributes
            .children
            .iter()
            .any(|node| node.as_element().is_some())
    });
    if empty {
        element.take_child("attributes");
    }
}

fn truncate(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// The last segment of a slash or backslash separated path, and what precedes it.
fn split_path(path: &str) -> (&str, &str) {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(index) => (&trimmed[index + 1..], &trimmed[..index]),
        None => (trimmed, ""),
    }
}

/// The name a container path is known by in Eclipse's Java Build Path page.
fn container_title(zh: bool, path: &str) -> String {
    let (id, rest) = path.split_once('/').unwrap_or((path, ""));
    let last = split_path(path).0;
    match id {
        "org.eclipse.jdt.junit.JUNIT_CONTAINER" => format!("JUnit {rest}"),
        "org.eclipse.jdt.USER_LIBRARY" if !rest.is_empty() => rest.to_owned(),
        "org.eclipse.jst.j2ee.internal.web.container" => {
            local(zh, "Web App Libraries", "Web 应用库")
        }
        "org.eclipse.jst.j2ee.internal.module.container" => local(zh, "EAR Libraries", "EAR 库"),
        "org.eclipse.jst.server.core.container" => {
            format!("{}{last}", local(zh, "Server Runtime: ", "服务器运行时："))
        }
        _ if !last.is_empty() => last.to_owned(),
        _ => path.to_owned(),
    }
}

fn entry(zh: bool, kind: &str, element: &Element) -> Value {
    let path = attribute(element, "path");
    let (title, detail) = match kind {
        LIBRARY => (container_title(zh, path), path.to_owned()),
        MODULE => (path.trim_start_matches('/').to_owned(), String::new()),
        _ => {
            let (name, parent) = split_path(path);
            (name.to_owned(), parent.to_owned())
        }
    };
    let mut value = json!({"kind":kind,"value":path,"title":truncate(&title,256)});
    if !detail.is_empty() && detail != title {
        value["detail"] = json!(truncate(&detail, 1024));
    }
    let mut attributes = Map::new();
    if nested(element, TEST).as_deref() == Some("true") {
        attributes.insert(TEST.into(), json!(true));
    }
    if kind != FOLDER && attribute(element, EXPORTED) == "true" {
        attributes.insert(EXPORTED.into(), json!(true));
    }
    if !attributes.is_empty() {
        value["attributes"] = Value::Object(attributes);
    }
    value
}

/// What one `.classpath` declares, in the terms of the page.
#[derive(Debug, Default)]
pub struct Classpath {
    pub roots: Vec<Value>,
    pub output: Option<String>,
    pub jre: Option<String>,
    pub dependencies: Vec<Value>,
    /// Containers of every `con` entry but the JRE, offered to other modules.
    pub containers: Vec<String>,
    /// False when an entry the contract cannot carry (a blank or overlong
    /// path, too many entries) would be lost by a rewrite: the module is
    /// then shown read-only.
    pub editable: bool,
}

fn parse(bytes: &[u8]) -> Result<Element, FunctionError> {
    let document = Element::parse(bytes).map_err(|_| fail())?;
    if document.name != "classpath" {
        return Err(fail());
    }
    Ok(document)
}

pub fn read(zh: bool, bytes: &[u8]) -> Result<Classpath, FunctionError> {
    let document = parse(bytes)?;
    let mut classpath = Classpath {
        editable: true,
        ..Classpath::default()
    };
    for element in document.children.iter().filter_map(XMLNode::as_element) {
        let role = role(element);
        let path = attribute(element, "path");
        if matches!(role, Role::Root | Role::Dependency(_))
            && (path.trim().is_empty() || path.len() > MAX_VALUE)
        {
            classpath.editable = false;
            continue;
        }
        match role {
            Role::Root => classpath.roots.push(entry(zh, FOLDER, element)),
            Role::Jre => classpath.jre = Some(path.to_owned()),
            Role::Output => classpath.output = Some(path.to_owned()),
            Role::Dependency(kind) => {
                if kind == LIBRARY && !classpath.containers.iter().any(|known| known == path) {
                    classpath.containers.push(path.to_owned());
                }
                classpath.dependencies.push(entry(zh, kind, element));
            }
            Role::Other => {}
        }
    }
    for list in [&mut classpath.roots, &mut classpath.dependencies] {
        if list.len() > MAX_ENTRIES {
            list.truncate(MAX_ENTRIES);
            classpath.editable = false;
        }
    }
    Ok(classpath)
}

/// Everything a module of the workspace may add besides files and folders.
pub struct Choices<'a> {
    /// Container paths used anywhere in the workspace.
    pub containers: &'a [String],
    /// Eclipse project names of the workspace's modules.
    pub modules: &'a [String],
}

/// The fields of a writable Eclipse module, in Sources, Paths and
/// Dependencies sections.
pub fn fields(
    zh: bool,
    classpath: &Classpath,
    name: &str,
    choices: &Choices<'_>,
    origin: &str,
) -> Vec<Value> {
    let sources = local(zh, "Sources", "源");
    let paths = local(zh, "Paths", "路径");
    let dependencies = local(zh, "Dependencies", "依赖");
    let test = local(zh, "Test", "测试");
    let roots = json!(classpath.roots);
    let mut jre_options = vec![
        json!({"value":JRE_CONTAINER,"title":local(zh,"Workspace default JRE","工作区默认 JRE")}),
    ];
    for environment in EXECUTION_ENVIRONMENTS {
        jre_options.push(json!({"value":format!("{JRE_CONTAINER}/{STANDARD_VM}/{environment}"),"title":environment}));
    }
    if let Some(jre) = &classpath.jre {
        if !jre_options
            .iter()
            .any(|option| option["value"] == jre.as_str())
        {
            jre_options.push(json!({"value":jre,"title":split_path(jre).0}));
        }
    }
    let mut library_choices: Vec<Value> = Vec::new();
    for path in KNOWN_CONTAINERS
        .iter()
        .copied()
        .chain(choices.containers.iter().map(String::as_str))
    {
        if library_choices.len() < MAX_CHOICES
            && path.len() <= MAX_VALUE
            && !library_choices.iter().any(|choice| choice["value"] == path)
        {
            library_choices.push(json!({"value":path,"title":truncate(&container_title(zh,path),256),"detail":truncate(path,1024)}));
        }
    }
    let module_choices: Vec<Value> = choices
        .modules
        .iter()
        .filter(|module| module.as_str() != name)
        .take(MAX_CHOICES)
        .map(|module| json!({"value":format!("/{module}"),"title":truncate(module,256)}))
        .collect();
    let mut kinds = vec![
        json!({"id":ARCHIVE,"title":local(zh,"JARs or ZIPs","JAR 或 ZIP"),"input":"file","extensions":["jar","zip"]}),
        json!({"id":CLASSES,"title":local(zh,"Class folder","类文件夹"),"input":"directory"}),
        json!({"id":LIBRARY,"title":local(zh,"Library","库"),"input":"choice","choices":library_choices}),
        json!({"id":MODULE,"title":local(zh,"Module dependency","模块依赖"),"input":"choice","choices":module_choices}),
    ];
    // Classpath variables are kept and reordered, but only Eclipse defines them.
    if classpath
        .dependencies
        .iter()
        .any(|entry| entry["kind"] == VARIABLE)
    {
        kinds.push(json!({"id":VARIABLE,"title":local(zh,"Classpath variable","类路径变量"),"input":"choice","choices":[]}));
    }
    let dependency_list = json!(classpath.dependencies);
    vec![
        json!({"id":ROOTS_FIELD,"title":local(zh,"Source folders","源文件夹"),"section":sources,"kind":"entryList",
            "description":local(zh,"Folders JDT compiles, in .classpath order. Mark a folder as Test to keep it out of the main classpath.","JDT 编译的文件夹，按 .classpath 顺序。标为“测试”的文件夹不进入主类路径。"),
            "value":roots,"effectiveValue":roots,"effectiveSource":origin,
            "entryKinds":[{"id":FOLDER,"title":local(zh,"Source folder","源文件夹"),"input":"directory"}],
            "attributes":[{"id":TEST,"title":test,"kind":"boolean"}]}),
        json!({"id":OUTPUT_FIELD,"title":local(zh,"Output folder","输出文件夹"),"section":paths,"kind":"text",
            "description":local(zh,"Where JDT writes compiled classes, relative to the module. Empty uses bin.","JDT 写出编译结果的位置，相对于模块。留空使用 bin。"),
            "value":classpath.output,"effectiveValue":classpath.output.clone().unwrap_or_else(||"bin".into()),"effectiveSource":origin}),
        json!({"id":JRE_FIELD,"title":"JRE","section":dependencies.clone(),"kind":"enum",
            "description":local(zh,"An execution environment compiles against the matching JDK of Toolchains.","执行环境对应“工具链”中匹配的 JDK。"),
            "value":classpath.jre,"effectiveValue":classpath.jre,"effectiveSource":origin,"options":jre_options}),
        json!({"id":DEPENDENCIES_FIELD,"title":dependencies,"section":local(zh,"Dependencies","依赖"),"kind":"entryList",
            "description":local(zh,"Archives, class folders, libraries and other modules this module compiles against, in classpath order. Export makes an entry visible to modules that depend on this one.","此模块编译所依赖的归档、类文件夹、库和其他模块，按类路径顺序。“导出”让依赖本模块的模块也能看到该项。"),
            "value":dependency_list,"effectiveValue":dependency_list,"effectiveSource":origin,
            "entryKinds":kinds,
            "attributes":[{"id":EXPORTED,"title":local(zh,"Export","导出"),"kind":"boolean"},{"id":TEST,"title":local(zh,"Test only","仅测试"),"kind":"boolean"}]}),
    ]
}

/// The dependencies of a Maven or Gradle module as JDT resolved them. A path
/// inside the workspace is shown relative to it: a class folder by its whole
/// relative path, since its last segment (`classes`) says little.
pub fn resolved_fields(zh: bool, root: &Path, paths: &[String], origin: &str) -> Vec<Value> {
    let entries: Vec<Value> = paths
        .iter()
        .filter(|path| !path.trim().is_empty() && path.len() <= MAX_VALUE)
        .take(MAX_ENTRIES)
        .map(|path| {
            let shown = crate::project_model::relative_inside(root, Path::new(path))
                .unwrap_or_else(|| path.clone());
            let kind = if archive(path) { ARCHIVE } else { CLASSES };
            let (title, detail) = if kind == ARCHIVE {
                let (name, parent) = split_path(&shown);
                (name.to_owned(), parent.to_owned())
            } else {
                (shown.clone(), String::new())
            };
            let mut value = json!({"kind":kind,"value":path,"title":truncate(&title,256)});
            if !detail.is_empty() {
                value["detail"] = json!(truncate(&detail, 1024));
            }
            value
        })
        .collect();
    let entries = json!(entries);
    vec![
        json!({"id":DEPENDENCIES_FIELD,"title":local(zh,"Dependencies","依赖"),"kind":"entryList",
        "description":local(zh,"Resolved by the build tool, including test dependencies. Change them in the build file.","由构建工具解析，包含测试依赖。请在构建文件中修改。"),
        "value":entries,"effectiveValue":entries,"effectiveSource":origin,
        "entryKinds":[{"id":ARCHIVE,"title":local(zh,"JARs or ZIPs","JAR 或 ZIP"),"input":"file"},{"id":CLASSES,"title":local(zh,"Class folder","类文件夹"),"input":"directory"}]}),
    ]
}

/// How a picked path is stored: relative to the module inside it, otherwise
/// absolute with forward slashes as Eclipse writes it. A value that is not an
/// absolute path is already in the file's terms.
fn stored_path(module: &Path, value: &str, inside_only: bool) -> Result<String, FunctionError> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Ok(value.to_owned());
    }
    let resolved = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let canonical = dunce::canonicalize(module).unwrap_or_else(|_| module.to_path_buf());
    // Compare as given and as resolved: a picked path that does not exist
    // yet cannot be resolved, and a module may sit behind a link.
    let relative = path
        .strip_prefix(module)
        .ok()
        .or_else(|| resolved.strip_prefix(&canonical).ok());
    if let Some(relative) = relative {
        let parts: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.is_empty() {
            return Err(outside());
        }
        return Ok(parts.join("/"));
    }
    if inside_only {
        return Err(outside());
    }
    Ok(resolved.to_string_lossy().replace('\\', "/"))
}

fn flag(entry: &Value, id: &str) -> Result<bool, FunctionError> {
    match entry
        .get("attributes")
        .and_then(|attributes| attributes.get(id))
    {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(on)) => Ok(*on),
        Some(_) => Err(fail()),
    }
}

/// Takes the unused existing entry of `kind` at `path`, or makes a new one.
fn take(pool: &mut Vec<Element>, kind: &str, path: &str) -> Element {
    if let Some(index) = pool.iter().position(|element| {
        attribute(element, "kind") == kind && attribute(element, "path") == path
    }) {
        return pool.remove(index);
    }
    let mut element = Element::new("classpathentry");
    element.attributes.insert("kind".into(), kind.into());
    element.attributes.insert("path".into(), path.into());
    element
}

fn list(values: &Map<String, Value>, id: &str) -> Result<Option<Vec<Value>>, FunctionError> {
    match values.get(id) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(entries)) if entries.len() <= MAX_ENTRIES => Ok(Some(entries.clone())),
        Some(_) => Err(fail()),
    }
}

/// Applies the confirmed fields of a module to its `.classpath`.
pub fn edit(
    bytes: &[u8],
    module: &Path,
    values: &Map<String, Value>,
) -> Result<Vec<u8>, FunctionError> {
    let mut document = parse(bytes)?;
    for id in values.keys() {
        if ![ROOTS_FIELD, OUTPUT_FIELD, JRE_FIELD, DEPENDENCIES_FIELD].contains(&id.as_str()) {
            return Err(fail());
        }
    }
    let managed =
        |element: &Element| matches!(role(element), Role::Root | Role::Jre | Role::Dependency(_));
    let current: Vec<Element> = document
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(|element| managed(element))
        .cloned()
        .collect();
    let mut pool = current.clone();
    let mut next = Vec::new();

    match list(values, ROOTS_FIELD)? {
        Some(entries) => {
            for entry in entries {
                if entry["kind"] != FOLDER {
                    return Err(fail());
                }
                let value = entry["value"].as_str().ok_or_else(fail)?;
                let path = if pool
                    .iter()
                    .any(|e| role(e) == Role::Root && attribute(e, "path") == value)
                {
                    value.to_owned()
                } else {
                    stored_path(module, value, true)?
                };
                let mut element = take(&mut pool, "src", &path);
                set_nested(&mut element, TEST, flag(&entry, TEST)?.then_some("true"));
                next.push(element);
            }
        }
        None => next.extend(current.iter().filter(|e| role(e) == Role::Root).cloned()),
    }
    match values.get(JRE_FIELD) {
        Some(Value::String(path))
            if path == JRE_CONTAINER || path.starts_with(&format!("{JRE_CONTAINER}/")) =>
        {
            let mut element = match pool.iter().position(|e| role(e) == Role::Jre) {
                Some(index) => pool.remove(index),
                None => take(&mut pool, "con", path),
            };
            element.attributes.insert("path".into(), path.clone());
            next.push(element);
        }
        Some(Value::Null) => {}
        Some(_) => return Err(fail()),
        None => next.extend(current.iter().filter(|e| role(e) == Role::Jre).cloned()),
    }
    match list(values, DEPENDENCIES_FIELD)? {
        Some(entries) => {
            for entry in entries {
                let kind = entry["kind"].as_str().ok_or_else(fail)?;
                let native = native_kind(kind)
                    .filter(|_| kind != FOLDER)
                    .ok_or_else(fail)?;
                let value = entry["value"].as_str().ok_or_else(fail)?;
                let known = pool
                    .iter()
                    .any(|e| attribute(e, "kind") == native && attribute(e, "path") == value);
                let path = match kind {
                    ARCHIVE | CLASSES if !known => stored_path(module, value, false)?,
                    MODULE if !value.starts_with('/') || value.len() < 2 => return Err(fail()),
                    LIBRARY
                        if value == JRE_CONTAINER
                            || value.starts_with(&format!("{JRE_CONTAINER}/")) =>
                    {
                        return Err(fail())
                    }
                    VARIABLE if !known => return Err(fail()),
                    _ => value.to_owned(),
                };
                let created = !pool
                    .iter()
                    .any(|e| attribute(e, "kind") == native && attribute(e, "path") == path);
                let mut element = take(&mut pool, native, &path);
                if created && kind == MODULE {
                    element
                        .attributes
                        .insert("combineaccessrules".into(), "false".into());
                }
                if flag(&entry, EXPORTED)? {
                    element.attributes.insert(EXPORTED.into(), "true".into());
                } else {
                    element.attributes.shift_remove(EXPORTED);
                }
                set_nested(&mut element, TEST, flag(&entry, TEST)?.then_some("true"));
                next.push(element);
            }
        }
        None => next.extend(
            current
                .iter()
                .filter(|e| matches!(role(e), Role::Dependency(_)))
                .cloned(),
        ),
    }

    // The new entries take the old ones' places in order, so comments
    // between entries stay near them; extra entries follow the last place.
    let mut next = next.into_iter();
    let last = document
        .children
        .iter()
        .rposition(|node| node.as_element().is_some_and(&managed));
    let mut children = Vec::with_capacity(document.children.len());
    for (index, node) in std::mem::take(&mut document.children)
        .into_iter()
        .enumerate()
    {
        let slot = node.as_element().is_some_and(&managed);
        let output = node.as_element().is_some_and(|e| role(e) == Role::Output);
        if output && last.is_none() {
            children.extend(next.by_ref().map(XMLNode::Element));
        }
        if slot {
            if let Some(element) = next.next() {
                children.push(XMLNode::Element(element));
            }
        } else {
            children.push(node);
        }
        if Some(index) == last {
            children.extend(next.by_ref().map(XMLNode::Element));
        }
    }
    children.extend(next.map(XMLNode::Element));
    document.children = children;

    match values.get(OUTPUT_FIELD) {
        None => {}
        Some(Value::String(path)) if !path.trim().is_empty() => {
            let path = stored_path(module, path.trim(), true)?;
            match document
                .children
                .iter_mut()
                .filter_map(XMLNode::as_mut_element)
                .find(|e| role(e) == Role::Output)
            {
                Some(element) => {
                    element.attributes.insert("path".into(), path);
                }
                None => {
                    let mut element = Element::new("classpathentry");
                    element.attributes.insert("kind".into(), "output".into());
                    element.attributes.insert("path".into(), path);
                    document.children.push(XMLNode::Element(element));
                }
            }
        }
        Some(Value::String(_)) | Some(Value::Null) => document
            .children
            .retain(|node| !node.as_element().is_some_and(|e| role(e) == Role::Output)),
        Some(_) => return Err(fail()),
    }
    crate::project_settings::write_classpath(bytes, &document)
}

/// The Eclipse project name of a module: `.project`'s name, else its folder.
pub fn project_name(module: &Path) -> String {
    let folder = module
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Java")
        .to_owned();
    let Some(bytes) = crate::project_model::read_bounded(&module.join(".project")) else {
        return folder;
    };
    Element::parse(bytes.as_slice())
        .ok()
        .and_then(|document| {
            document
                .get_child("name")
                .and_then(|name| name.get_text())
                .map(|name| name.trim().to_owned())
        })
        .filter(|name| !name.is_empty() && name.len() < MAX_VALUE)
        .unwrap_or(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSPATH: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- before -->
<classpath>
	<classpathentry kind="src" path="src"/>
	<classpathentry kind="src" output="bin-test" path="test"><attributes><attribute name="test" value="true"/></attributes></classpathentry>
	<classpathentry kind="con" path="org.eclipse.jdt.launching.JRE_CONTAINER/org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType/JavaSE-17"/>
	<!-- libraries -->
	<classpathentry kind="lib" path="lib/core.jar" sourcepath="lib/core-sources.jar"><attributes><attribute name="javadoc_location" value="https://example.test/docs"/></attributes></classpathentry>
	<classpathentry exported="true" kind="con" path="org.eclipse.jdt.USER_LIBRARY/Shared Libs"/>
	<classpathentry combineaccessrules="false" kind="src" path="/common"/>
	<classpathentry kind="var" path="TOOLS_HOME/lib/tools.jar"/>
	<classpathentry kind="output" path="bin"/>
</classpath>
"#;

    fn values(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_classpath_reads_as_source_folders_output_jre_and_dependencies() {
        let classpath = read(false, CLASSPATH.as_bytes()).unwrap();
        assert!(classpath.editable);
        assert_eq!(
            json!(classpath.roots),
            json!([{"kind":"folder","value":"src","title":"src"},{"kind":"folder","value":"test","title":"test","attributes":{"test":true}}])
        );
        assert_eq!(classpath.output.as_deref(), Some("bin"));
        assert!(classpath.jre.as_deref().unwrap().ends_with("JavaSE-17"));
        let kinds: Vec<_> = classpath
            .dependencies
            .iter()
            .map(|entry| {
                format!(
                    "{}:{}",
                    entry["kind"].as_str().unwrap(),
                    entry["title"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "archive:core.jar",
                "library:Shared Libs",
                "module:common",
                "variable:tools.jar"
            ]
        );
        assert_eq!(
            classpath.dependencies[1]["attributes"],
            json!({"exported":true})
        );
        assert_eq!(
            classpath.containers,
            ["org.eclipse.jdt.USER_LIBRARY/Shared Libs"]
        );
        let fields = fields(
            false,
            &classpath,
            "app",
            &Choices {
                containers: &classpath.containers,
                modules: &["app".into(), "common".into(), "web".into()],
            },
            "app/.classpath",
        );
        let dependencies = &fields[3];
        let modules = &dependencies["entryKinds"][3]["choices"];
        assert_eq!(
            modules,
            &json!([{"value":"/common","title":"common"},{"value":"/web","title":"web"}])
        );
        assert_eq!(dependencies["entryKinds"][4]["id"], VARIABLE);
    }

    #[test]
    fn an_edit_reorders_flags_and_adds_entries_keeping_what_the_page_does_not_show() {
        let module = std::env::temp_dir();
        let picked = module.join("vendor").join("extra.jar");
        let edited = edit(
            CLASSPATH.as_bytes(),
            &module,
            &values(json!({
                "roots":[{"kind":"folder","value":"test","attributes":{"test":true}},{"kind":"folder","value":"src","attributes":{"test":true}}],
                "jre":"org.eclipse.jdt.launching.JRE_CONTAINER",
                "dependencies":[
                    {"kind":"module","value":"/common","attributes":{"exported":true}},
                    {"kind":"archive","value":"lib/core.jar","attributes":{"test":true}},
                    {"kind":"variable","value":"TOOLS_HOME/lib/tools.jar"},
                    {"kind":"archive","value":picked.to_str().unwrap()},
                    {"kind":"library","value":"org.eclipse.jdt.junit.JUNIT_CONTAINER/5"},
                    {"kind":"module","value":"/web"}
                ],
                "output":"build/classes"
            })),
        )
        .unwrap();
        let text = String::from_utf8(edited.clone()).unwrap();
        for kept in [
            "<!-- before -->",
            "<!-- libraries -->",
            "core-sources.jar",
            "javadoc_location",
            "output=\"bin-test\"",
        ] {
            assert!(text.contains(kept), "{kept}: {text}");
        }
        assert!(!text.contains("Shared Libs"), "{text}");
        let classpath = read(false, &edited).unwrap();
        let roots: Vec<_> = classpath
            .roots
            .iter()
            .map(|e| (e["value"].as_str().unwrap(), flag(e, TEST).unwrap()))
            .collect();
        assert_eq!(roots, [("test", true), ("src", true)]);
        assert_eq!(classpath.jre.as_deref(), Some(JRE_CONTAINER));
        assert_eq!(classpath.output.as_deref(), Some("build/classes"));
        let dependencies: Vec<_> = classpath
            .dependencies
            .iter()
            .map(|e| {
                (
                    e["value"].as_str().unwrap().to_owned(),
                    flag(e, EXPORTED).unwrap(),
                    flag(e, TEST).unwrap(),
                )
            })
            .collect();
        let extra = stored_path(&module, picked.to_str().unwrap(), false).unwrap();
        assert_eq!(extra, "vendor/extra.jar");
        assert_eq!(
            dependencies,
            [
                ("/common".to_owned(), true, false),
                ("lib/core.jar".to_owned(), false, true),
                ("TOOLS_HOME/lib/tools.jar".to_owned(), false, false),
                (extra, false, false),
                (
                    "org.eclipse.jdt.junit.JUNIT_CONTAINER/5".to_owned(),
                    false,
                    false
                ),
                ("/web".to_owned(), false, false),
            ]
        );
        assert!(
            text.contains(r#"combineaccessrules="false" kind="src" path="/web""#)
                || text.contains("path=\"/web\""),
            "{text}"
        );
        // Fields the edit does not name stay as they are.
        let unchanged = edit(&edited, &module, &values(json!({"output":null}))).unwrap();
        let after = read(false, &unchanged).unwrap();
        assert_eq!(after.output, None);
        assert_eq!(json!(after.dependencies), json!(classpath.dependencies));
        assert_eq!(json!(after.roots), json!(classpath.roots));
    }

    #[test]
    fn an_edit_refuses_what_the_page_never_offers() {
        let module = std::env::temp_dir().join("module");
        let outside = std::env::temp_dir().join("elsewhere");
        for bad in [
            json!({"roots":[{"kind":"folder","value":outside.to_str().unwrap()}]}),
            json!({"dependencies":[{"kind":"variable","value":"NEW_VAR/x.jar"}]}),
            json!({"dependencies":[{"kind":"library","value":JRE_CONTAINER}]}),
            json!({"dependencies":[{"kind":"folder","value":"src"}]}),
            json!({"jre":"org.example.OTHER"}),
            json!({"unknown":true}),
        ] {
            assert!(
                edit(CLASSPATH.as_bytes(), &module, &values(bad.clone())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_entry_the_contract_cannot_carry_makes_the_module_read_only() {
        let classpath = read(false, br#"<classpath><classpathentry kind="src" path=""/><classpathentry kind="lib" path="a.jar"/></classpath>"#).unwrap();
        assert!(!classpath.editable);
        assert_eq!(classpath.dependencies.len(), 1);
    }

    #[test]
    fn resolved_dependencies_are_archives_and_class_folders() {
        let root = std::env::temp_dir().join("workspace");
        let classes = root.join("core").join("target").join("classes");
        let fields = resolved_fields(
            true,
            &root,
            &[
                "/repo/a/1/a-1.jar".into(),
                classes.to_string_lossy().into_owned(),
                String::new(),
            ],
            "pom.xml",
        );
        assert_eq!(
            fields[0]["value"],
            json!([{"kind":"archive","value":"/repo/a/1/a-1.jar","title":"a-1.jar","detail":"/repo/a/1"},{"kind":"classes","value":classes.to_string_lossy(),"title":"core/target/classes"}])
        );
    }
}
