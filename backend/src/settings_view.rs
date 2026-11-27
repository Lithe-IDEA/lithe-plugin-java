//! Settings-region preview views rendered natively by the Lithe host.
//!
//! The plugin manifest attaches `settings.view` to two `settingRegions`:
//! - `language` ("Languages & Frameworks -> Java"): draws four adaptive cards
//!   visualizing the live effective and draft values of `heapMaxMB`,
//!   `languageServerHome`, `logLevel`, `projectModelDetection`, and
//!   `servletContainerHome`.
//! - `compiler` ("Build, Execution, Deployment -> Java Compiler"): draws the
//!   three-tier compiler & bytecode precedence diagram above the
//!   "Read compiler settings" action.
//!
//! All coordinates are in view-box units scaled 1:1 to the width reported by
//! the host (`width` + `on: ["resize"]`). When an older host omits `width`, the
//! scene falls back to the narrowest settings pane width and omits `on`.

use lithe_plugin_sdk::{Context, FunctionError, PluginError, Value};
use serde_json::{json, Map};

/// Function ID declared in `plugin.json` under `settingRegions[].view.function`.
pub const SETTINGS_VIEW_FUNCTION_ID: &str = "settings.view";

/// Error code returned when a host request carries malformed view parameters
/// or names an unknown surface.
pub const INVALID_PARAMS_CODE: &str = "settings_view.invalid_params";

/// Diagnostic category reported via `degraded` when a setting value has an
/// unexpected type or out-of-range value and falls back to its default.
pub const UNRECOGNIZED_SETTING_CATEGORY: &str = "settings_view.setting_unrecognized";

/// Error codes owned by this module for disjointness checks in `lib.rs`.
pub const VIEW_ERROR_CODES: &[&str] = &[INVALID_PARAMS_CODE];

/// Diagnostic categories owned by this module for disjointness checks in `lib.rs`.
pub const VIEW_CATEGORIES: &[&str] = &[UNRECOGNIZED_SETTING_CATEGORY];

const LANGUAGE_REGION: &str = "language";
const COMPILER_REGION: &str = "compiler";

/// Default width when an older host omits `width` on the Java language page
/// (613pt pane in the 900px minimum settings window).
pub const DEFAULT_LANGUAGE_WIDTH: f64 = 610.0;

/// Default width when an older host omits `width` on the Java compiler page
/// (344pt detail pane in `buildKindPage` inside the 900px settings window).
pub const DEFAULT_COMPILER_WIDTH: f64 = 340.0;

/// Minimum width of a single preview card on the Java language page.
pub const MIN_CARD_WIDTH: f64 = 292.0;

/// Minimum width of the Java compiler precedence scene.
pub const MIN_COMPILER_WIDTH: f64 = 330.0;

/// Horizontal and vertical spacing between grid cards.
pub const CARD_GAP: f64 = 10.0;

const HEADER_H: f64 = 28.0;
const TITLE_Y: f64 = 22.0;
const NOTE_Y: f64 = 41.0;
const BOX_X: f64 = 10.0;
const BOX_Y: f64 = 50.0;
const BOX_H: f64 = 90.0;
const CARD_H: f64 = BOX_Y + BOX_H + BOX_X;

const CAPTION_SIZE: f64 = 11.0;
const STRONG_SIZE: f64 = 13.0;
const MONO_ADVANCE: f64 = 7.0;

const HEAP_MIN_MB: i64 = 2048;
const HEAP_MAX_MB: i64 = 16384;

/// Handles a `settings.view` invocation from the host.
///
/// Ordinary redraws only present state. An explicit check inspects local
/// server layouts; full content verification belongs to runtime preparation.
pub fn view_function(
    session: &crate::settings_session::SettingsSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let request = parse_request(&params)?;
    let check = request.region == LANGUAGE_REGION
        && request.event == "click"
        && params["node"] == "check-jdtls";
    if !check && !matches!(request.event, "open" | "settings" | "resize") {
        return Ok(json!({}));
    }
    let words = words_for_language(request.language);
    let (settings, unrecognized) = read_settings(request.effective, request.draft);
    if unrecognized > 0 {
        let _ = context.diagnostics().degraded(
            SETTINGS_VIEW_FUNCTION_ID,
            UNRECOGNIZED_SETTING_CATEGORY,
            Some(unrecognized),
        );
    }
    let canvas = match request.region {
        LANGUAGE_REGION => language_canvas(&settings, words, request.width),
        COMPILER_REGION => compiler_canvas(words, request.width),
        _ => {
            return Err(PluginError::new(
                INVALID_PARAMS_CODE,
                "Settings view only supports the Java language and compiler regions.",
            )
            .into());
        }
    };
    let mut view = Vec::new();
    if request.region == LANGUAGE_REGION {
        let zh = request.language.to_ascii_lowercase().starts_with("zh");
        let configured = settings.language_server_home.as_str();
        let status = context
            .workspace_context()
            .and_then(|ws| session.service_environment(ws, configured));
        let label = |text: &str| json!({"type":"label","text":text});
        if let Some(lines) = status {
            view.push(label(if zh {
                "上次启动检查采用的环境"
            } else {
                "Environment selected at last launch check"
            }));
            view.extend(lines.iter().map(|line| label(line)));
        } else {
            view.push(label(if zh {
                "当前选择尚无启动检查结果"
            } else {
                "No launch check recorded for this selection"
            }));
        }
        view.push(json!({"type":"button","id":"check-jdtls","text":if zh { "检查本机语言服务器目录" } else { "Check local language server folders" }}));
        if check {
            let lines = crate::managed_runtimes::check_local_servers(configured, zh, &|| {
                context.check_cancelled().map_err(Into::into)
            })?;
            view.extend(lines.iter().map(|line| label(line)));
        }
        view.push(json!({"type":"tool","tool":crate::launch::SERVER_RUNTIME_TOOL}));
    }
    view.push(canvas);
    Ok(json!({ "view": view }))
}

struct ParsedRequest<'a> {
    event: &'a str,
    region: &'a str,
    language: &'a str,
    width: Option<f64>,
    effective: &'a Map<String, Value>,
    draft: &'a Map<String, Value>,
}

fn parse_request(params: &Value) -> Result<ParsedRequest<'_>, PluginError> {
    let invalid = |msg: &str| PluginError::new(INVALID_PARAMS_CODE, msg);
    let object = params
        .as_object()
        .ok_or_else(|| invalid("Settings view event parameters must be a JSON object."))?;
    let event = object
        .get("event")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid("Settings view event parameters require a non-empty event name."))?;
    let region = match object.get("surface") {
        Some(surface) => {
            let surface_obj = surface
                .as_object()
                .ok_or_else(|| invalid("Settings view surface must be an object."))?;
            if let Some(region_id) = surface_obj.get("settingRegionID").and_then(Value::as_str) {
                if region_id == LANGUAGE_REGION || region_id == COMPILER_REGION {
                    region_id
                } else {
                    return Err(invalid("Unknown settings region identifier."));
                }
            } else if surface_obj.is_empty() {
                LANGUAGE_REGION
            } else {
                return Err(invalid(
                    "Settings view surface must name a declared settingRegionID.",
                ));
            }
        }
        None => LANGUAGE_REGION,
    };
    let language = object
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("en");
    let width = match object.get("width") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let w = value
                .as_f64()
                .filter(|w| w.is_finite() && *w >= 1.0)
                .ok_or_else(|| invalid("Settings view width must be a positive number."))?;
            Some(w.floor())
        }
    };
    static EMPTY_MAP: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);
    let (effective, draft) = match object.get("settings") {
        None | Some(Value::Null) => (&*EMPTY_MAP, &*EMPTY_MAP),
        Some(settings) => {
            let settings_obj = settings
                .as_object()
                .ok_or_else(|| invalid("Settings view settings payload must be an object."))?;
            let eff = match settings_obj.get("effective") {
                None | Some(Value::Null) => &*EMPTY_MAP,
                Some(v) => v
                    .as_object()
                    .ok_or_else(|| invalid("settings.effective must be an object."))?,
            };
            let dr = match settings_obj.get("draft") {
                None | Some(Value::Null) => &*EMPTY_MAP,
                Some(v) => v
                    .as_object()
                    .ok_or_else(|| invalid("settings.draft must be an object."))?,
            };
            (eff, dr)
        }
    };
    Ok(ParsedRequest {
        event,
        region,
        language,
        width,
        effective,
        draft,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogLevel {
    All,
    Info,
    Warning,
    Severe,
}

impl LogLevel {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "ALL" => Some(Self::All),
            "INFO" => Some(Self::Info),
            "WARNING" => Some(Self::Warning),
            "SEVERE" => Some(Self::Severe),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct JavaSettings {
    heap_max_mb: i64,
    language_server_home: String,
    log_level: LogLevel,
    project_model_detection: bool,
    servlet_container_home: String,
    changed: Vec<&'static str>,
}

impl JavaSettings {
    fn is_changed(&self, id: &str) -> bool {
        self.changed.contains(&id)
    }

    fn any_changed(&self) -> bool {
        !self.changed.is_empty()
    }
}

fn read_settings(
    effective: &Map<String, Value>,
    draft: &Map<String, Value>,
) -> (JavaSettings, u64) {
    let mut changed = Vec::new();
    let mut unrecognized = 0_u64;

    let mut read_field =
        |id: &'static str, default: Value, parse: &dyn Fn(&Value) -> Option<Value>| -> Value {
            let saved = effective.get(id).unwrap_or(&default);
            if let Some(d) = draft.get(id) {
                if d != saved {
                    changed.push(id);
                }
            }
            let shown = draft.get(id).unwrap_or(saved);
            if let Some(parsed) = parse(shown) {
                parsed
            } else {
                unrecognized += 1;
                default
            }
        };

    let heap_val = read_field("heapMaxMB", json!(HEAP_MIN_MB), &|v| {
        v.as_i64()
            .filter(|n| (HEAP_MIN_MB..=HEAP_MAX_MB).contains(n))
            .map(|n| json!(n))
    });
    let heap_max_mb = heap_val.as_i64().unwrap_or(HEAP_MIN_MB);

    let ls_home_val = read_field("languageServerHome", json!(""), &|v| {
        v.as_str().map(|s| json!(s.trim()))
    });
    let language_server_home = ls_home_val.as_str().unwrap_or("").to_string();

    let log_val = read_field("logLevel", json!("WARNING"), &|v| {
        v.as_str()
            .filter(|s| LogLevel::parse(s).is_some())
            .map(|s| json!(s))
    });
    let log_level =
        LogLevel::parse(log_val.as_str().unwrap_or("WARNING")).unwrap_or(LogLevel::Warning);

    let model_val = read_field("projectModelDetection", json!(true), &|v| {
        v.as_bool().map(|b| json!(b))
    });
    let project_model_detection = model_val.as_bool().unwrap_or(true);

    let servlet_val = read_field("servletContainerHome", json!(""), &|v| {
        v.as_str().map(|s| json!(s.trim()))
    });
    let servlet_container_home = servlet_val.as_str().unwrap_or("").to_string();

    (
        JavaSettings {
            heap_max_mb,
            language_server_home,
            log_level,
            project_model_detection,
            servlet_container_home,
            changed,
        },
        unrecognized,
    )
}

struct Words {
    preview_title: &'static str,
    preview_label: &'static str,
    follows_options: &'static str,
    includes_draft: &'static str,
    on: &'static str,
    off: &'static str,

    heap_title: &'static str,
    heap_note_managed: &'static str,
    heap_note_custom: &'static str,
    home_auto_value: &'static str,

    log_title: &'static str,
    log_options: [&'static str; 4],
    log_notes: [&'static str; 4],
    log_filtered: &'static str,

    model_title: &'static str,
    model_on_note: &'static str,
    model_off_note: &'static str,
    model_on_status: &'static str,
    model_off_status: &'static str,

    servlet_title: &'static str,
    servlet_auto_pill: &'static str,
    servlet_custom_pill: &'static str,
    servlet_auto_note: &'static str,
    servlet_custom_note: &'static str,
    servlet_auto_order: &'static str,
    servlet_provides: &'static str,

    compiler_title: &'static str,
    compiler_label: &'static str,
    compiler_precedence_hint: &'static str,
    compiler_step1_title: &'static str,
    compiler_step1_chip: &'static str,
    compiler_step1_detail: &'static str,
    compiler_step2_title: &'static str,
    compiler_step2_chip: &'static str,
    compiler_step2_detail: &'static str,
    compiler_step3_title: &'static str,
    compiler_step3_chip: &'static str,
    compiler_step3_source: &'static str,
    compiler_step3_detail: &'static str,
    compiler_footer: &'static str,
}

const ZH: Words = Words {
    preview_title: "效果预览",
    preview_label: "Java 语言服务器与工程模型设置预览",
    follows_options: "示例代码，随下方选项即时变化",
    includes_draft: "含未应用的修改",
    on: "开",
    off: "关",

    heap_title: "JDT LS 内存与目录",
    heap_note_managed: "自动准备 JDT LS 1.61.0 与独立 JDK 25",
    heap_note_custom: "使用指定的 JDT LS 1.61.0 目录",
    home_auto_value: "自动查找 / 受管 JDT LS 1.61.0",

    log_title: "语言服务器日志级别",
    log_options: ["全部消息", "信息", "警告", "仅错误"],
    log_notes: [
        "记录最详细的 JDT LS 跟踪、索引与信息日志",
        "记录项目导入、构建状态与警告错误",
        "只向 jdt.log 写入警告与错误，减少写盘",
        "仅记录严重错误，磁盘写入最少",
    ],
    log_filtered: "已过滤",

    model_title: "自动识别工程模型",
    model_on_note: "内存补全 Eclipse WTP、IDEA 与 WEB-INF/lib",
    model_off_note: "仅读取 Maven、Gradle 或基础 .classpath",
    model_on_status: "已加入内存类路径，不修改工程文件",
    model_off_status: "未识别 WEB-INF/lib / WTP / IDEA 依赖",

    servlet_title: "Servlet 容器文件夹",
    servlet_auto_pill: "自动查找",
    servlet_custom_pill: "已指定",
    servlet_auto_note: "依次查找 CATALINA_HOME、TOMCAT_HOME 与常见位置",
    servlet_custom_note: "使用指定的 Tomcat / Jetty 安装目录",
    servlet_auto_order: "CATALINA_HOME → TOMCAT_HOME → 常见位置",
    servlet_provides: "提供 javax.servlet（lib/servlet-api.jar）",

    compiler_title: "编译设置生效顺序",
    compiler_label: "Java 编译器设置生效优先级预览",
    compiler_precedence_hint: "优先级从上到下",
    compiler_step1_title: "模块编译配置",
    compiler_step1_chip: "原生可写",
    compiler_step1_detail: "按模块设置 source / target / release 与告警",
    compiler_step2_title: "构建工具模型",
    compiler_step2_chip: "构建同步",
    compiler_step2_detail: "由 Maven / Gradle 插件读取并同步到 JDT",
    compiler_step3_title: "项目 JDK 默认级别",
    compiler_step3_chip: "回退默认",
    compiler_step3_source: "工具链 › JDK (java)",
    compiler_step3_detail: "未单独覆盖时使用项目 JDK 的目标字节码",
    compiler_footer: "点击下方「读取编译设置」查看或编辑真实工程配置",
};

const EN: Words = Words {
    preview_title: "Preview",
    preview_label: "Preview of Java language server and project model settings",
    follows_options: "Sample preview; follows the options below",
    includes_draft: "Includes changes not applied yet",
    on: "On",
    off: "Off",

    heap_title: "JDT LS Heap & Folder",
    heap_note_managed: "Managed JDT LS 1.61.0 with private JDK 25",
    heap_note_custom: "Uses custom JDT LS 1.61.0 directory",
    home_auto_value: "Auto / managed JDT LS 1.61.0",

    log_title: "Server Log Level",
    log_options: ["All", "Info", "Warnings", "Errors"],
    log_notes: [
        "Writes detailed JDT LS trace, indexing and info logs",
        "Writes project import, status and warning/error logs",
        "Writes warnings and errors to jdt.log to reduce disk I/O",
        "Writes severe errors only for minimal disk writes",
    ],
    log_filtered: "filtered",

    model_title: "Detect Project Models",
    model_on_note: "Adds WTP, IDEA and WEB-INF/lib jars in memory",
    model_off_note: "Reads Maven, Gradle or plain .classpath only",
    model_on_status: "Added to memory classpath; files untouched",
    model_off_status: "Cannot resolve WEB-INF/lib, WTP or IDEA",

    servlet_title: "Servlet Container",
    servlet_auto_pill: "Auto",
    servlet_custom_pill: "Custom",
    servlet_auto_note: "Checks CATALINA_HOME, TOMCAT_HOME and standard paths",
    servlet_custom_note: "Uses configured Tomcat or Jetty installation",
    servlet_auto_order: "CATALINA_HOME → TOMCAT_HOME → standard folders",
    servlet_provides: "Provides javax.servlet (lib/servlet-api.jar)",

    compiler_title: "Compiler Precedence",
    compiler_label: "Preview of Java compiler settings precedence",
    compiler_precedence_hint: "Top wins",
    compiler_step1_title: "Module JDT Preferences",
    compiler_step1_chip: "Editable",
    compiler_step1_detail: "Per-module source, target, release & warnings",
    compiler_step2_title: "Build System Model",
    compiler_step2_chip: "Synced",
    compiler_step2_detail: "Read by Maven / Gradle plugins for JDT LS",
    compiler_step3_title: "Project JDK Default",
    compiler_step3_chip: "Default",
    compiler_step3_source: "Toolchains › JDK (java)",
    compiler_step3_detail: "Fallback bytecode target when not overridden",
    compiler_footer: "Click Read compiler settings below to inspect live files",
};

fn words_for_language(language: &str) -> &'static Words {
    if language.trim().to_ascii_lowercase().starts_with("zh") {
        &ZH
    } else {
        &EN
    }
}

struct Node(Map<String, Value>);

impl Node {
    fn of(kind: &str) -> Self {
        let mut node = Map::new();
        node.insert("type".into(), kind.into());
        Self(node)
    }

    fn set(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    fn at(self, key: &str, value: f64) -> Self {
        self.set(key, round(value))
    }

    fn fill(self, color: impl Into<Value>) -> Self {
        self.set("fill", color)
    }

    fn stroke(self, color: impl Into<Value>, width: f64) -> Self {
        self.set("stroke", color).at("width", width)
    }

    fn radius(self, radius: f64) -> Self {
        self.at("radius", radius)
    }

    fn opacity(self, opacity: f64) -> Self {
        self.at("opacity", opacity)
    }

    fn color(self, color: impl Into<Value>) -> Self {
        self.set("color", color)
    }

    fn max_width(self, width: f64) -> Self {
        self.at("maxWidth", width.max(1.0))
    }

    fn align(self, align: &str) -> Self {
        self.set("align", align)
    }
}

impl From<Node> for Value {
    fn from(node: Node) -> Self {
        Value::Object(node.0)
    }
}

fn round(value: f64) -> Value {
    let tenths = (value * 10.0).round();
    if tenths % 10.0 == 0.0 {
        json!((tenths / 10.0) as i64)
    } else {
        json!(tenths / 10.0)
    }
}

fn fmt(value: f64) -> String {
    round(value).to_string()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Node {
    Node::of("rect").at("x", x).at("y", y).at("w", w).at("h", h)
}

fn text(x: f64, y: f64, value: &str, style: &str) -> Node {
    Node::of("text")
        .at("x", x)
        .at("y", y)
        .set("text", value)
        .set("style", style)
}

fn runs(x: f64, y: f64, pieces: &[(&str, Value)], style: &str) -> Node {
    let runs: Vec<Value> = pieces
        .iter()
        .map(|(text, color)| json!({ "text": text, "color": color }))
        .collect();
    Node::of("text")
        .at("x", x)
        .at("y", y)
        .set("runs", runs)
        .set("style", style)
}

fn path(d: String) -> Node {
    Node::of("path").set("d", d)
}

fn group(x: f64, y: f64, children: Vec<Node>) -> Node {
    let children: Vec<Value> = children.into_iter().map(Value::from).collect();
    Node::of("group")
        .at("x", x)
        .at("y", y)
        .set("children", children)
}

fn pair(light: &str, dark: &str) -> Value {
    json!({ "light": light, "dark": dark })
}

fn token(name: &str) -> Value {
    Value::from(name)
}

fn keyword() -> Value {
    pair("#0033B3", "#CF8E6D")
}

fn type_color() -> Value {
    pair("#00627A", "#56A8F5")
}

fn comment_color() -> Value {
    token("$textMuted")
}

fn editor_bg() -> Value {
    pair("#FFFFFF", "#1E1F22")
}

fn text_width(value: &str, size: f64) -> f64 {
    let ems: f64 = value
        .chars()
        .map(|character| match character {
            ' ' => 0.3,
            '\u{2E80}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}' => 1.0,
            'A'..='Z' | '0'..='9' | 'm' | 'w' => 0.66,
            'i' | 'j' | 'l' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 0.3,
            _ => 0.55,
        })
        .sum();
    ems * size
}

fn pill(x: f64, y: f64, words: &str, fill: Value, ink: Value) -> (Node, f64) {
    let width = text_width(words, CAPTION_SIZE) + 16.0;
    let node = group(
        x,
        y,
        vec![
            rect(0.0, 0.0, width, 20.0).radius(10.0).fill(fill),
            text(width / 2.0, 14.0, words, "caption")
                .align("center")
                .color(ink),
        ],
    );
    (node, width)
}

fn column(x: f64, col: usize) -> f64 {
    x + col as f64 * MONO_ADVANCE
}

fn code(x: f64, y: f64, pieces: &[(&str, Value)]) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut index = 0;
    for (piece, color) in pieces {
        for character in piece.chars() {
            if !character.is_whitespace() {
                nodes.push(
                    text(column(x, index), y, &character.to_string(), "monoSmall")
                        .color(color.clone()),
                );
            }
            index += 1;
        }
    }
    nodes
}

fn squiggle(x0: f64, x1: f64, y: f64) -> String {
    let mut d = format!("M{} {}", fmt(x0), fmt(y));
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        x = (x + 2.0).min(x1);
        let dy = if up { -1.5 } else { 1.5 };
        d.push_str(&format!(" L{} {}", fmt(x), fmt(y + dy)));
        up = !up;
    }
    d
}

fn tail(value: &str, room: usize) -> String {
    let count = value.chars().count();
    if count <= room || room < 2 {
        return value.to_string();
    }
    let kept: String = value.chars().skip(count - room + 1).collect();
    format!("…{kept}")
}

struct LanguageLayout {
    width: f64,
    columns: usize,
    card_w: f64,
}

impl LanguageLayout {
    fn new(width: Option<f64>) -> Self {
        let width = width.unwrap_or(DEFAULT_LANGUAGE_WIDTH).max(MIN_CARD_WIDTH);
        let columns = if width >= 2.0 * MIN_CARD_WIDTH + CARD_GAP {
            2
        } else {
            1
        };
        let card_w = ((width - (columns - 1) as f64 * CARD_GAP) / columns as f64).floor();
        let effective_width = if columns == 2 {
            card_w * 2.0 + CARD_GAP
        } else {
            card_w
        };
        Self {
            width: effective_width,
            columns,
            card_w,
        }
    }

    fn box_w(&self) -> f64 {
        self.card_w - 2.0 * BOX_X
    }

    fn places(&self) -> ([(f64, f64); 4], f64) {
        let mut places = [(0.0, 0.0); 4];
        let mut y = HEADER_H;
        for (index, place) in places.iter_mut().enumerate() {
            let col = index % self.columns;
            if index > 0 && col == 0 {
                y += CARD_H + CARD_GAP;
            }
            *place = (col as f64 * (self.card_w + CARD_GAP), y);
        }
        (places, y + CARD_H)
    }
}

struct Card<'a> {
    title: &'a str,
    value: String,
    lit: bool,
    note: &'a str,
    changed: bool,
    preview: Vec<Node>,
}

fn language_canvas(settings: &JavaSettings, words: &'static Words, width: Option<f64>) -> Value {
    let layout = LanguageLayout::new(width);
    let box_w = layout.box_w();
    let changed = settings.any_changed();
    let mut nodes = vec![
        text(0.0, 17.0, words.preview_title, "strong"),
        text(
            layout.width,
            17.0,
            if changed {
                words.includes_draft
            } else {
                words.follows_options
            },
            "caption",
        )
        .align("end")
        .color(token(if changed { "$accent" } else { "$textMuted" }))
        .max_width(layout.width - text_width(words.preview_title, STRONG_SIZE) - 16.0),
    ];

    let custom_ls_home = !settings.language_server_home.is_empty();
    let heap_lit = settings.heap_max_mb > HEAP_MIN_MB || custom_ls_home;
    let heap_note = if custom_ls_home {
        words.heap_note_custom
    } else {
        words.heap_note_managed
    };

    let log_idx = match settings.log_level {
        LogLevel::All => 0,
        LogLevel::Info => 1,
        LogLevel::Warning => 2,
        LogLevel::Severe => 3,
    };

    let model_on = settings.project_model_detection;
    let servlet_custom = !settings.servlet_container_home.is_empty();

    let cards = [
        Card {
            title: words.heap_title,
            value: format!("{} MB", settings.heap_max_mb),
            lit: heap_lit,
            note: heap_note,
            changed: settings.is_changed("heapMaxMB") || settings.is_changed("languageServerHome"),
            preview: heap_preview(settings, words, box_w),
        },
        Card {
            title: words.log_title,
            value: words.log_options[log_idx].to_string(),
            lit: true,
            note: words.log_notes[log_idx],
            changed: settings.is_changed("logLevel"),
            preview: log_preview(settings.log_level, words, box_w),
        },
        Card {
            title: words.model_title,
            value: if model_on { words.on } else { words.off }.to_string(),
            lit: model_on,
            note: if model_on {
                words.model_on_note
            } else {
                words.model_off_note
            },
            changed: settings.is_changed("projectModelDetection"),
            preview: model_preview(model_on, words, box_w),
        },
        Card {
            title: words.servlet_title,
            value: if servlet_custom {
                words.servlet_custom_pill
            } else {
                words.servlet_auto_pill
            }
            .to_string(),
            lit: servlet_custom,
            note: if servlet_custom {
                words.servlet_custom_note
            } else {
                words.servlet_auto_note
            },
            changed: settings.is_changed("servletContainerHome"),
            preview: servlet_preview(settings, words, box_w),
        },
    ];

    let (places, height) = layout.places();
    for (card, (x, y)) in cards.into_iter().zip(places) {
        nodes.push(group(x, y, draw_card(card, layout.card_w)));
    }
    let scene_w = width.unwrap_or(DEFAULT_LANGUAGE_WIDTH).max(MIN_CARD_WIDTH);
    let mut canvas = json!({
        "type": "canvas",
        "id": "java-language-preview",
        "label": words.preview_label,
        "height": height.ceil() as u64,
        "viewBox": [0, 0, scene_w, height],
        "nodes": nodes.into_iter().map(Value::from).collect::<Vec<_>>(),
    });
    if width.is_some() {
        canvas["on"] = json!(["resize"]);
    }
    canvas
}

fn draw_card(card: Card, card_w: f64) -> Vec<Node> {
    // Lit pills draw $text over $selection to keep >=4.5:1 contrast in dark mode.
    let (fill, ink) = if card.lit {
        (token("$selection"), token("$text"))
    } else {
        (token("$badge"), token("$textSecondary"))
    };
    let (_, pill_w) = pill(0.0, 0.0, &card.value, fill.clone(), ink.clone());
    let pill_x = card_w - 12.0 - pill_w;
    let (pill_node, _) = pill(pill_x, 8.0, &card.value, fill, ink);
    let frame = rect(0.0, 0.0, card_w, CARD_H)
        .radius(8.0)
        .fill(token("$surface"));
    let mut nodes = vec![
        if card.changed {
            frame.stroke(token("$accent"), 1.2)
        } else {
            frame.stroke(token("$line"), 1.0)
        },
        text(12.0, TITLE_Y, card.title, "strong").max_width(pill_x - 12.0 - 18.0),
        pill_node,
        text(12.0, NOTE_Y, card.note, "caption")
            .color(token("$textSecondary"))
            .max_width(card_w - 24.0),
        rect(BOX_X, BOX_Y, card_w - 2.0 * BOX_X, BOX_H)
            .radius(6.0)
            .fill(editor_bg())
            .stroke(token("$line"), 1.0),
        group(BOX_X, BOX_Y, card.preview),
    ];
    if card.changed {
        nodes.push(
            rect(pill_x - 11.0, 15.0, 6.0, 6.0)
                .radius(3.0)
                .fill(token("$accent")),
        );
    }
    nodes
}

fn heap_preview(settings: &JavaSettings, words: &Words, box_w: f64) -> Vec<Node> {
    let inner_w = box_w - 20.0;
    let ratio = (settings.heap_max_mb - HEAP_MIN_MB) as f64 / (HEAP_MAX_MB - HEAP_MIN_MB) as f64;
    let fill_w = (inner_w * (0.16 + 0.84 * ratio.clamp(0.0, 1.0))).min(inner_w);
    let path_room = ((inner_w / MONO_ADVANCE).floor() as usize)
        .saturating_sub(6)
        .max(8);
    let home_text = if settings.language_server_home.is_empty() {
        words.home_auto_value.to_string()
    } else {
        tail(&settings.language_server_home, path_room)
    };
    let home_color = if settings.language_server_home.is_empty() {
        token("$textSecondary")
    } else {
        token("$text")
    };
    vec![
        runs(
            10.0,
            21.0,
            &[
                ("jdk-25/bin/java ", token("$textMuted")),
                ("-Xms256m ", token("$textSecondary")),
                (&format!("-Xmx{}m", settings.heap_max_mb), token("$accent")),
            ],
            "monoSmall",
        )
        .max_width(inner_w),
        runs(
            10.0,
            40.0,
            &[("home: ", token("$textMuted")), (&home_text, home_color)],
            "monoSmall",
        )
        .max_width(inner_w),
        rect(10.0, 53.0, inner_w, 8.0)
            .radius(4.0)
            .fill(token("$badge")),
        rect(10.0, 53.0, fill_w, 8.0)
            .radius(4.0)
            .fill(token("$accent")),
        text(10.0, 77.0, "2048 MB", "caption").color(token("$textMuted")),
        text(box_w - 10.0, 77.0, "16384 MB", "caption")
            .align("end")
            .color(token("$textMuted")),
    ]
}

fn log_preview(level: LogLevel, words: &Words, box_w: f64) -> Vec<Node> {
    let inner_w = box_w - 20.0;
    let mut nodes = vec![text(10.0, 17.0, "${logDir}/jdt.log", "caption")
        .color(token("$textMuted"))
        .max_width(inner_w)];
    let lines = [
        (
            "[FINE]   ",
            "workspace/executeCommand",
            token("$accent"),
            matches!(level, LogLevel::All),
        ),
        (
            "[INFO]   ",
            "Java workspace initialized",
            token("$text"),
            matches!(level, LogLevel::All | LogLevel::Info),
        ),
        (
            "[WARN]   ",
            "Fallback classpath applied",
            token("$warning"),
            matches!(level, LogLevel::All | LogLevel::Info | LogLevel::Warning),
        ),
        ("[SEVERE] ", "JDT LS build failed", token("$error"), true),
    ];
    for (index, (tag, msg, tag_color, active)) in lines.into_iter().enumerate() {
        let y = 34.0 + index as f64 * 16.0;
        let line_node = if active {
            runs(
                10.0,
                y,
                &[(tag, tag_color), (msg, token("$textSecondary"))],
                "monoSmall",
            )
            .max_width(inner_w)
        } else {
            let suffix = format!(" ({})", words.log_filtered);
            runs(
                10.0,
                y,
                &[
                    (tag, token("$textMuted")),
                    (msg, token("$textMuted")),
                    (&suffix, token("$textMuted")),
                ],
                "monoSmall",
            )
            .max_width(inner_w)
            .opacity(0.42)
        };
        nodes.push(line_node);
    }
    nodes
}

fn model_preview(enabled: bool, words: &Words, box_w: f64) -> Vec<Node> {
    const CODE_TOP: f64 = 20.0;
    const CODE_X: f64 = 28.0;
    const LINE_H: f64 = 17.0;
    let line1 = vec![
        ("import ", keyword()),
        ("com.acme.WebUtil", type_color()),
        (";", token("$text")),
    ];
    let line2 = vec![("// WEB-INF/lib/util.jar (WTP/IDEA)", comment_color())];
    let mut nodes = Vec::new();
    if enabled {
        nodes.push(
            rect(0.0, CODE_TOP + LINE_H - 12.0, box_w, LINE_H)
                .fill(token("$success"))
                .opacity(0.08),
        );
    }
    for (index, pieces) in [line1, line2].iter().enumerate() {
        let y = CODE_TOP + index as f64 * LINE_H;
        nodes.push(
            text(CODE_X - 8.0, y, &(index + 1).to_string(), "monoSmall")
                .align("end")
                .color(token("$textMuted")),
        );
        nodes.extend(code(CODE_X, y, pieces));
    }
    let bar_w = box_w - 12.0;
    if enabled {
        nodes.push(group(
            6.0,
            58.0,
            vec![
                rect(0.0, 0.0, bar_w, 22.0)
                    .radius(5.0)
                    .fill(pair("#E6F4EA", "#253A2B")),
                path("M9 11.5 L12.5 15 L18.5 8".into())
                    .stroke(token("$success"), 1.6)
                    .set("cap", "round"),
                text(25.0, 14.5, words.model_on_status, "caption")
                    .color(token("$success"))
                    .max_width(bar_w - 31.0),
            ],
        ));
    } else {
        nodes.push(
            path(squiggle(
                column(CODE_X, 7),
                column(CODE_X, 23),
                CODE_TOP + 3.0,
            ))
            .stroke(token("$error"), 1.0),
        );
        nodes.push(group(
            6.0,
            58.0,
            vec![
                rect(0.0, 0.0, bar_w, 22.0)
                    .radius(5.0)
                    .fill(pair("#FFF4DE", "#3B3124")),
                path("M13 5 L19 16 H7 Z".into()).fill(token("$warning")),
                path("M13 8.6 V12 M13 14 V14.1".into())
                    .stroke("#FFFFFF", 1.4)
                    .set("cap", "round"),
                text(25.0, 14.5, words.model_off_status, "caption")
                    .color(token("$textSecondary"))
                    .max_width(bar_w - 31.0),
            ],
        ));
    }
    nodes
}

fn servlet_preview(settings: &JavaSettings, words: &Words, box_w: f64) -> Vec<Node> {
    let inner_w = box_w - 20.0;
    let custom = !settings.servlet_container_home.is_empty();
    let path_room = ((inner_w / MONO_ADVANCE).floor() as usize)
        .saturating_sub(8)
        .max(8);
    let mut nodes = vec![runs(
        10.0,
        21.0,
        &[
            ("import ", keyword()),
            ("javax.servlet.*", type_color()),
            (";", token("$text")),
        ],
        "monoSmall",
    )
    .max_width(inner_w)];
    if custom {
        let trimmed = tail(&settings.servlet_container_home, path_room);
        nodes.push(
            runs(
                10.0,
                40.0,
                &[
                    ("server: ", token("$textMuted")),
                    (&trimmed, token("$text")),
                ],
                "monoSmall",
            )
            .max_width(inner_w),
        );
    } else {
        nodes.push(
            runs(
                10.0,
                40.0,
                &[
                    ("order: ", token("$textMuted")),
                    (words.servlet_auto_order, token("$textSecondary")),
                ],
                "caption",
            )
            .max_width(inner_w),
        );
    }
    let bar_w = box_w - 12.0;
    let (bar_fill, bar_ink) = if custom {
        (token("$selection"), token("$text"))
    } else {
        (token("$badge"), token("$textSecondary"))
    };
    nodes.push(group(
        6.0,
        58.0,
        vec![
            rect(0.0, 0.0, bar_w, 22.0).radius(5.0).fill(bar_fill),
            text(10.0, 14.5, words.servlet_provides, "caption")
                .color(bar_ink)
                .max_width(bar_w - 20.0),
        ],
    ));
    nodes
}

fn compiler_canvas(words: &'static Words, width: Option<f64>) -> Value {
    const RAIL: f64 = 11.0;
    const DOT: f64 = 20.0;
    const STEP_X: f64 = 30.0;
    const STEP_H: f64 = 64.0;
    const STEP_GAP: f64 = 10.0;
    const TITLE_MID: f64 = 17.0;
    const LINE_X: f64 = 12.0;
    const FOOTER_Y: f64 = HEADER_H + 3.0 * (STEP_H + STEP_GAP) + 4.0;
    const HEIGHT: f64 = FOOTER_Y + 20.0;

    let scene_w = width
        .unwrap_or(DEFAULT_COMPILER_WIDTH)
        .max(MIN_COMPILER_WIDTH);
    let step_w = scene_w - STEP_X;
    let title_w = text_width(words.compiler_title, STRONG_SIZE);
    let mut nodes = vec![
        text(0.0, 17.0, words.compiler_title, "strong"),
        text(scene_w, 17.0, words.compiler_precedence_hint, "caption")
            .align("end")
            .color(token("$textMuted"))
            .max_width(scene_w - title_w - 12.0),
    ];

    struct Step<'a> {
        number: u8,
        title: &'a str,
        chip: &'a str,
        lit: bool,
        source: &'a str,
        detail: &'a str,
    }

    let steps = [
        Step {
            number: 1,
            title: words.compiler_step1_title,
            chip: words.compiler_step1_chip,
            lit: true,
            source: ".settings/org.eclipse.jdt.core.prefs",
            detail: words.compiler_step1_detail,
        },
        Step {
            number: 2,
            title: words.compiler_step2_title,
            chip: words.compiler_step2_chip,
            lit: false,
            source: "pom.xml / build.gradle (.kts)",
            detail: words.compiler_step2_detail,
        },
        Step {
            number: 3,
            title: words.compiler_step3_title,
            chip: words.compiler_step3_chip,
            lit: false,
            source: words.compiler_step3_source,
            detail: words.compiler_step3_detail,
        },
    ];

    for (index, step) in steps.iter().enumerate() {
        let y = HEADER_H + index as f64 * (STEP_H + STEP_GAP);
        if index > 0 {
            let above = y - STEP_GAP - STEP_H + TITLE_MID + DOT / 2.0 + 4.0;
            let below = y + TITLE_MID - DOT / 2.0 - 4.0;
            nodes.push(
                path(format!("M{RAIL} {above} V{below}"))
                    .stroke(token("$line"), 2.0)
                    .set("cap", "round"),
            );
        }
        let frame = if step.lit {
            rect(0.0, 0.0, step_w, STEP_H)
                .radius(8.0)
                .fill(token("$selection"))
                .stroke(token("$accent"), 1.3)
        } else {
            rect(0.0, 0.0, step_w, STEP_H)
                .radius(8.0)
                .fill(token("$surface"))
                .stroke(token("$line"), 1.0)
        };
        let (dot_fill, dot_ink) = if step.lit {
            (token("$accent"), Value::from("#FFFFFF"))
        } else {
            (token("$badge"), token("$textSecondary"))
        };
        let (chip_fill, chip_ink) = if step.lit {
            (token("$selection"), token("$text"))
        } else {
            (token("$badge"), token("$textSecondary"))
        };
        let (_, chip_w) = pill(0.0, 0.0, step.chip, chip_fill.clone(), chip_ink.clone());
        let chip_x = step_w - 10.0 - chip_w;
        let (chip_node, _) = pill(chip_x, TITLE_MID - 10.0, step.chip, chip_fill, chip_ink);

        let card_nodes = vec![
            frame,
            text(LINE_X, TITLE_MID + 4.5, step.title, "strong").max_width(chip_x - LINE_X - 10.0),
            chip_node,
            text(LINE_X, 40.0, step.source, "monoSmall")
                .color(token("$text"))
                .max_width(step_w - 2.0 * LINE_X),
            text(LINE_X, 55.0, step.detail, "caption")
                .color(token("$textSecondary"))
                .max_width(step_w - 2.0 * LINE_X),
        ];
        nodes.push(group(
            0.0,
            y,
            vec![
                rect(RAIL - DOT / 2.0, TITLE_MID - DOT / 2.0, DOT, DOT)
                    .radius(DOT / 2.0)
                    .fill(dot_fill),
                text(RAIL, TITLE_MID + 4.0, &step.number.to_string(), "caption")
                    .align("center")
                    .color(dot_ink),
                group(STEP_X, 0.0, card_nodes),
            ],
        ));
    }

    nodes.push(
        text(0.0, FOOTER_Y + 13.0, words.compiler_footer, "caption")
            .color(token("$textMuted"))
            .max_width(scene_w),
    );

    let mut canvas = json!({
        "type": "canvas",
        "id": "java-compiler-preview",
        "label": words.compiler_label,
        "height": HEIGHT.ceil() as u64,
        "viewBox": [0, 0, scene_w, HEIGHT],
        "nodes": nodes.into_iter().map(Value::from).collect::<Vec<_>>(),
    });
    if width.is_some() {
        canvas["on"] = json!(["resize"]);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUNDING: f64 = 0.1;

    fn event_json(
        kind: &str,
        region: &str,
        language: &str,
        width: Option<f64>,
        effective: Value,
        draft: Value,
    ) -> Value {
        let mut ev = json!({
            "event": kind,
            "instance": "i1",
            "surface": { "settingRegionID": region },
            "language": language,
            "settings": { "effective": effective, "draft": draft },
        });
        if let Some(w) = width {
            ev["width"] = json!(w);
        }
        ev
    }

    fn render_region(
        kind: &str,
        region: &str,
        language: &str,
        width: Option<f64>,
        effective: Value,
        draft: Value,
    ) -> Value {
        let ev = event_json(kind, region, language, width, effective, draft);
        let parsed = parse_request(&ev).unwrap();
        let (settings, _) = read_settings(parsed.effective, parsed.draft);
        let words = words_for_language(parsed.language);
        match parsed.region {
            LANGUAGE_REGION => language_canvas(&settings, words, parsed.width),
            COMPILER_REGION => compiler_canvas(words, parsed.width),
            _ => unreachable!(),
        }
    }

    fn walk<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        out.push(node);
        for key in ["children", "nodes"] {
            if let Some(children) = node.get(key).and_then(Value::as_array) {
                for child in children {
                    walk(child, out);
                }
            }
        }
    }

    fn right_edge(nodes: &[Value], x0: f64) -> f64 {
        let mut edge: f64 = 0.0;
        for node in nodes {
            let x = x0 + node.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let is_code_char = node["style"] == "monoSmall"
                && node["text"]
                    .as_str()
                    .is_some_and(|t| t.chars().count() == 1);
            let reach = match (
                node["type"].as_str(),
                node.get("align").and_then(Value::as_str),
            ) {
                (Some("rect"), _) => x + node["w"].as_f64().unwrap(),
                (Some("text"), Some("end")) => x,
                (Some("text"), None) if is_code_char => x + MONO_ADVANCE,
                (Some("text"), None) => node
                    .get("maxWidth")
                    .and_then(Value::as_f64)
                    .map_or(0.0, |w| x + w),
                _ => 0.0,
            };
            edge = edge.max(reach);
            for key in ["children", "nodes"] {
                if let Some(children) = node.get(key).and_then(Value::as_array) {
                    edge = edge.max(right_edge(children, x));
                }
            }
        }
        edge
    }

    fn words_of(canvas: &Value) -> String {
        let mut all = Vec::new();
        walk(canvas, &mut all);
        let mut out = String::new();
        let mut cell: Option<(f64, f64)> = None;
        for node in all {
            let text = node.get("text").and_then(Value::as_str);
            if let Some(ch) = text.filter(|t| {
                t.chars().count() == 1
                    && node["style"] == "monoSmall"
                    && node.get("align").is_none()
            }) {
                let (x, y) = (node["x"].as_f64().unwrap(), node["y"].as_f64().unwrap());
                match cell {
                    Some((lx, ly)) if (ly - y).abs() < 0.01 => {
                        let cols = ((x - lx) / MONO_ADVANCE).round() as usize;
                        out.push_str(&" ".repeat(cols.saturating_sub(1)));
                    }
                    Some(_) => out.push('\n'),
                    None => {}
                }
                out.push_str(ch);
                cell = Some((x, y));
                continue;
            }
            if cell.take().is_some() {
                out.push('\n');
            }
            if let Some(t) = text {
                out.push_str(t);
                out.push('\n');
            }
            if let Some(runs) = node.get("runs").and_then(Value::as_array) {
                for run in runs {
                    out.push_str(run["text"].as_str().unwrap());
                }
                out.push('\n');
            }
        }
        out
    }

    fn check_limits(canvas: &Value) {
        let label = canvas["label"].as_str().unwrap();
        assert!(!label.trim().is_empty() && label.chars().count() <= 120);
        let height = canvas["height"].as_u64().expect("integer canvas height");
        assert!((1..=2000).contains(&height));
        let mut scene = Vec::new();
        walk(canvas, &mut scene);
        assert!(scene.len() <= 2000);
        assert!(serde_json::to_vec(canvas).unwrap().len() <= 256 * 1024);
        for node in scene {
            if let Some(text) = node.get("text").and_then(Value::as_str) {
                assert!(text.chars().count() <= 1024);
                assert!(!text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'));
            }
            if let Some(runs) = node.get("runs").and_then(Value::as_array) {
                assert!(!runs.is_empty() && runs.len() <= 64);
            }
        }
    }

    #[test]
    fn without_a_width_scenes_omit_resize_and_fit_smallest_pane() {
        for (region, default_w) in [
            (LANGUAGE_REGION, DEFAULT_LANGUAGE_WIDTH),
            (COMPILER_REGION, DEFAULT_COMPILER_WIDTH),
        ] {
            for lang in ["en", "zh-Hans"] {
                let canvas = render_region("open", region, lang, None, json!({}), json!({}));
                check_limits(&canvas);
                assert_eq!(canvas["viewBox"][2].as_f64().unwrap(), default_w);
                assert_eq!(
                    canvas["height"].as_f64().unwrap(),
                    canvas["viewBox"][3].as_f64().unwrap().ceil()
                );
                assert!(canvas.get("pan").is_none());
                assert!(canvas.get("on").is_none());
            }
        }
    }

    #[test]
    fn adaptive_widths_include_resize_and_stay_within_view_box() {
        let widths = [260.0, 330.0, 500.0, 593.0, 594.0, 610.0, 900.0, 1400.0];
        let cases = [
            (json!({}), json!({})),
            (
                json!({
                    "heapMaxMB": 4096,
                    "languageServerHome": "/opt/jdtls-1.61.0",
                    "logLevel": "ALL",
                    "projectModelDetection": false,
                    "servletContainerHome": "/opt/apache-tomcat-9.0"
                }),
                json!({ "heapMaxMB": 8192, "logLevel": "SEVERE" }),
            ),
        ];
        for (region, min_w) in [
            (LANGUAGE_REGION, MIN_CARD_WIDTH),
            (COMPILER_REGION, MIN_COMPILER_WIDTH),
        ] {
            for lang in ["en", "zh-Hans"] {
                for (effective, draft) in &cases {
                    for w in widths {
                        let canvas = render_region(
                            "resize",
                            region,
                            lang,
                            Some(w),
                            effective.clone(),
                            draft.clone(),
                        );
                        check_limits(&canvas);
                        let expected_w = w.max(min_w);
                        assert_eq!(
                            canvas["viewBox"][2].as_f64().unwrap(),
                            expected_w,
                            "{region} {lang} {w}"
                        );
                        assert_eq!(canvas["on"], json!(["resize"]));
                        assert!(canvas.get("pan").is_none());
                        let reach = right_edge(std::slice::from_ref(&canvas), 0.0);
                        assert!(
                            reach <= expected_w + ROUNDING,
                            "{region} {lang} at {w}: {reach} > {expected_w}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn language_cards_switch_between_one_and_two_columns_at_594() {
        let two_min = 2.0 * MIN_CARD_WIDTH + CARD_GAP;
        assert_eq!(two_min, 594.0);
        for lang in ["en", "zh-Hans"] {
            for (width, expected_columns) in [(593.0, 1), (594.0, 2), (900.0, 2), (330.0, 1)] {
                let canvas = render_region(
                    "open",
                    LANGUAGE_REGION,
                    lang,
                    Some(width),
                    json!({}),
                    json!({}),
                );
                let cards: Vec<&Value> = canvas["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|n| n["type"] == "group")
                    .collect();
                assert_eq!(cards.len(), 4);
                let second_col = cards
                    .iter()
                    .filter(|c| c["x"].as_f64().unwrap() > 0.0)
                    .count();
                assert_eq!(
                    second_col,
                    if expected_columns == 2 { 2 } else { 0 },
                    "{lang} at {width}"
                );
                for card in cards {
                    let children = card["children"].as_array().unwrap();
                    let frame_w = children[0]["w"].as_f64().unwrap();
                    let reach = right_edge(children, 0.0);
                    assert!(
                        reach <= frame_w + ROUNDING,
                        "{lang} at {width}: child reach {reach} > frame {frame_w}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_title_keeps_its_room_beside_its_pill_in_the_narrowest_card() {
        for words in [&EN, &ZH] {
            let mut pairs: Vec<(&str, String)> = vec![
                (words.heap_title, "16384 MB".to_string()),
                (words.model_title, words.on.to_string()),
                (words.model_title, words.off.to_string()),
                (words.servlet_title, words.servlet_auto_pill.to_string()),
                (words.servlet_title, words.servlet_custom_pill.to_string()),
            ];
            for opt in words.log_options {
                pairs.push((words.log_title, opt.to_string()));
            }
            for (title, pill_text) in pairs {
                let pill_w = text_width(&pill_text, CAPTION_SIZE) + 16.0;
                let pill_x = MIN_CARD_WIDTH - 12.0 - pill_w;
                let room = pill_x - 12.0 - 18.0;
                let title_w = text_width(title, STRONG_SIZE);
                assert!(
                    title_w <= room,
                    "{title} ({title_w}) exceeds room ({room}) beside {pill_text}"
                );
            }
        }
    }

    #[test]
    fn drafts_update_language_preview_immediately_and_mark_unsaved() {
        let clean = render_region(
            "open",
            LANGUAGE_REGION,
            "en",
            Some(700.0),
            json!({ "heapMaxMB": 2048, "logLevel": "WARNING", "projectModelDetection": true }),
            json!({}),
        );
        let clean_text = words_of(&clean);
        assert!(clean_text.contains("-Xmx2048m"));
        assert!(clean_text.contains("Added to memory classpath"));
        assert!(!clean_text.contains("Includes changes not applied yet"));

        let dirty = render_region(
            "settings",
            LANGUAGE_REGION,
            "en",
            Some(700.0),
            json!({ "heapMaxMB": 2048, "logLevel": "WARNING", "projectModelDetection": true }),
            json!({
                "heapMaxMB": 8192,
                "languageServerHome": "/Users/dev/jdtls-1.61.0",
                "logLevel": "SEVERE",
                "projectModelDetection": false,
                "servletContainerHome": "/opt/tomcat9"
            }),
        );
        let dirty_text = words_of(&dirty);
        assert!(dirty_text.contains("-Xmx8192m"));
        assert!(dirty_text.contains("jdtls-1.61.0"));
        assert!(dirty_text.contains("Errors"));
        assert!(dirty_text.contains("Cannot resolve WEB-INF/lib"));
        assert!(dirty_text.contains("tomcat9"));
        assert!(dirty_text.contains("Includes changes not applied yet"));
    }

    #[test]
    fn close_and_non_redraw_events_return_empty_and_invalid_params_fail() {
        let close_ev = event_json(
            "close",
            LANGUAGE_REGION,
            "zh-Hans",
            None,
            json!({}),
            json!({}),
        );
        let parsed = parse_request(&close_ev).unwrap();
        assert!(!matches!(parsed.event, "open" | "settings" | "resize"));

        let err = parse_request(&json!({ "surface": { "settingRegionID": "language" } }))
            .err()
            .unwrap();
        assert_eq!(err.code(), INVALID_PARAMS_CODE);

        let bad_region_ev = event_json("open", "other", "en", None, json!({}), json!({}));
        let bad_region = parse_request(&bad_region_ev).err().unwrap();
        assert_eq!(bad_region.code(), INVALID_PARAMS_CODE);
    }

    #[test]
    fn unrecognized_setting_values_are_counted_and_fall_back_to_defaults() {
        let ev = event_json(
            "settings",
            LANGUAGE_REGION,
            "en",
            Some(610.0),
            json!({ "heapMaxMB": 512, "logLevel": "TRACE" }),
            json!({ "projectModelDetection": "yes" }),
        );
        let parsed = parse_request(&ev).unwrap();
        let (settings, unrecognized) = read_settings(parsed.effective, parsed.draft);
        assert_eq!(unrecognized, 3);
        assert_eq!(settings.heap_max_mb, HEAP_MIN_MB);
        assert_eq!(settings.log_level, LogLevel::Warning);
        assert!(settings.project_model_detection);
    }
}
