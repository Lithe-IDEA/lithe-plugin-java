//! Read-only qualification of the Java plugin's managed dependencies.
//!
//! The host owns downloads, archive verification, atomic installation and
//! cache lifetime. This function only chooses installations that can run the
//! plugin's pinned extensions. It never downloads, writes to an installation,
//! or executes a candidate program. The project `java` tool is never changed.

use crate::jdtls::{self, SearchEnvironment, ServerInstallation, SERVER_HOME_SETTING};
use crate::launch::SERVER_RUNTIME_TOOL;
use lithe_plugin_sdk::{Context, FunctionError, PluginError, Value};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Backend hook used by the host's generic managed-runtime installer.
pub const RESOLVE_FUNCTION_ID: &str = "java.resolveRuntimes";
/// Dependency ID of the full JDK used only by the language server.
pub const JDK_ID: &str = "jdk";
/// Dependency ID of the upstream Eclipse JDT LS installation.
pub const JDTLS_ID: &str = "jdtls";
/// The Java feature release verified with the pinned JDT LS and extensions.
pub const JAVA_FEATURE: u64 = 25;
/// Version of the full Microsoft Build of OpenJDK installed when none qualifies.
pub const JDK_VERSION: &str = "25.0.5+8";
/// The request does not match the plugin's declared dependency contract.
pub const INVALID_PARAMS: &str = "runtime.invalid_params";
/// An explicitly selected server is not the verified upstream distribution.
pub const SERVER_INCOMPATIBLE: &str = "runtime.server_incompatible";
/// The explicitly selected service tool is not a usable full Java 25 JDK.
pub const JDK_INVALID: &str = "runtime.jdk_invalid";
/// The host has not prepared the runtime map required for this launch.
pub const NOT_PREPARED: &str = "runtime.not_prepared";
/// The compiled upstream fingerprint is invalid; reinstalling repairs it.
pub const PROFILE_INVALID: &str = "runtime.profile_invalid";
/// Error categories reported by the SDK for runtime resolution.
pub const ERROR_CODES: &[&str] = &[
    INVALID_PARAMS,
    SERVER_INCOMPATIBLE,
    JDK_INVALID,
    NOT_PREPARED,
    PROFILE_INVALID,
];

const PROFILE: &str = include_str!("../../third_party/jdtls/runtime-files.json");
const MAX_DISCOVERED_SERVERS: usize = 32;
const MAX_RELEASE_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
struct PinnedFile {
    path: PathBuf,
    size: u64,
    sha256: String,
}

#[derive(Debug)]
struct ServerProfile {
    version: String,
    files: Vec<PinnedFile>,
    configurations: Vec<(String, PinnedFile)>,
}

impl ServerProfile {
    fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let file = |value: &Value| -> Option<PinnedFile> {
            let path = PathBuf::from(value.get("path")?.as_str()?);
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return None;
            }
            let sha256 = value.get("sha256")?.as_str()?;
            if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            Some(PinnedFile {
                path,
                size: value.get("size")?.as_u64()?,
                sha256: sha256.to_owned(),
            })
        };
        let files: Vec<_> = value
            .get("files")?
            .as_array()?
            .iter()
            .map(file)
            .collect::<Option<_>>()?;
        if files.is_empty() || files.len() > 1024 {
            return None;
        }
        let configurations = value
            .get("configurations")?
            .as_object()?
            .iter()
            .map(|(name, value)| Some((name.clone(), file(value)?)))
            .collect::<Option<_>>()?;
        Some(Self {
            version: value.get("version")?.as_str()?.to_owned(),
            files,
            configurations,
        })
    }

    fn pinned() -> Result<Self, PluginError> {
        Self::parse(PROFILE).ok_or_else(|| {
            PluginError::new(
                PROFILE_INVALID,
                "Java Support cannot read its runtime requirements. Reinstall the plugin.",
            )
        })
    }
}

fn invalid() -> PluginError {
    PluginError::new(
        INVALID_PARAMS,
        "Java Support received invalid runtime preparation parameters.",
    )
}

/// Error for an explicit installation that does not match the verified set.
pub fn incompatible_server() -> PluginError {
    PluginError::new(
        SERVER_INCOMPATIBLE,
        "The selected language server does not match the verified JDT LS 1.61.0 files. \
Choose that release or clear Language server folder so Lithe can prepare it automatically.",
    )
}

/// Error used instead of silently rediscovering a server after preparation.
pub fn not_prepared() -> PluginError {
    PluginError::new(
        NOT_PREPARED,
        "The Java language service environment is not ready. Retry Java Support environment preparation.",
    )
}

fn absolute(value: Option<&Value>) -> Option<PathBuf> {
    value
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn release_value<'a>(release: &'a str, key: &str) -> Option<&'a str> {
    release.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then(|| value.trim().trim_matches('"'))
    })
}

fn java_25(version: &str) -> bool {
    !version.contains("-ea")
        && version
            .split(['.', '+', '-', '_'])
            .next()
            .and_then(|feature| feature.parse::<u64>().ok())
            == Some(JAVA_FEATURE)
}

fn regular_nonempty(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

/// Checks native launcher headers without executing any candidate Java.
/// The release file alone can survive a partial extraction or point at a
/// different architecture; the Java and javac launchers must agree with it.
fn native_launcher(path: &Path, os: &str, arch: &str) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 4096 {
        return false;
    }
    #[cfg(unix)]
    if os != "windows" {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut bytes = [0u8; 512];
    if file.read_exact(&mut bytes).is_err() {
        return false;
    }
    let little32 = |offset| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("four header bytes"),
        )
    };
    match os {
        "linux" => {
            bytes[..6] == [0x7f, b'E', b'L', b'F', 2, 1]
                && u16::from_le_bytes([bytes[18], bytes[19]])
                    == if arch == "arm64" { 183 } else { 62 }
        }
        "macos" => {
            let cpu = if arch == "arm64" {
                0x0100000c
            } else {
                0x01000007
            };
            if bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe] {
                return little32(4) == cpu;
            }
            let width = match bytes[..4] {
                [0xca, 0xfe, 0xba, 0xbe] => 20,
                [0xca, 0xfe, 0xba, 0xbf] => 32,
                _ => return false,
            };
            let count = u32::from_be_bytes(bytes[4..8].try_into().expect("four header bytes"));
            count <= 12
                && (0..count as usize).any(|index| {
                    let start = 8 + index * width;
                    u32::from_be_bytes(
                        bytes[start..start + 4]
                            .try_into()
                            .expect("four header bytes"),
                    ) == cpu
                })
        }
        "windows" => {
            if bytes[..2] != *b"MZ" {
                return false;
            }
            let offset = little32(60) as u64;
            let mut header = [0u8; 6];
            offset <= 1024 * 1024
                && file.seek(SeekFrom::Start(offset)).is_ok()
                && file.read_exact(&mut header).is_ok()
                && header[..4] == *b"PE\0\0"
                && u16::from_le_bytes([header[4], header[5]])
                    == if arch == "arm64" { 0xaa64 } else { 0x8664 }
        }
        _ => false,
    }
}

/// Qualifies a full JDK 25, returning its canonical home and release version.
/// A JRE (no javac/jmods), early-access build, wrong architecture, empty or
/// non-native launcher and partial archive are not reusable.
pub fn inspect_jdk(home: &Path, os: &str, arch: &str) -> Option<(PathBuf, String)> {
    let home = dunce::canonicalize(home).ok()?;
    let mut release = String::new();
    File::open(home.join("release"))
        .ok()?
        .take(MAX_RELEASE_BYTES)
        .read_to_string(&mut release)
        .ok()?;
    let version = release_value(&release, "JAVA_VERSION")?;
    if !java_25(version) {
        return None;
    }
    let reported_arch = release_value(&release, "OS_ARCH")?;
    if !matches!(
        (arch, reported_arch),
        ("arm64", "aarch64" | "arm64") | ("x86_64", "amd64" | "x86_64" | "x64")
    ) {
        return None;
    }
    let reported_os = release_value(&release, "OS_NAME")?;
    if !matches!(
        (os, reported_os),
        ("linux", "Linux") | ("macos", "Darwin" | "macOS") | ("windows", "Windows" | "Windows_NT")
    ) {
        return None;
    }
    let suffix = if os == "windows" { ".exe" } else { "" };
    if !["java", "javac"]
        .iter()
        .all(|name| native_launcher(&home.join(format!("bin/{name}{suffix}")), os, arch))
        || !regular_nonempty(&home.join("lib/modules"))
        || !regular_nonempty(&home.join("jmods/java.base.jmod"))
    {
        return None;
    }
    Some((home, version.to_owned()))
}

fn matches_file(home: &Path, pin: &PinnedFile) -> bool {
    let path = home.join(&pin.path);
    if !fs::metadata(&path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == pin.size) {
        return false;
    }
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut file = file.take(pin.size.saturating_add(1));
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        match file.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => hash.update(&bytes[..count]),
            Err(_) => return false,
        }
    }
    format!("{:x}", hash.finalize()) == pin.sha256
}

fn inspect_server_with(
    home: &Path,
    os: &str,
    arch: &str,
    profile: &ServerProfile,
    check_cancelled: &dyn Fn() -> Result<(), FunctionError>,
) -> Result<Option<ServerInstallation>, FunctionError> {
    let Some(configuration) = jdtls::configuration_name(os, arch) else {
        return Ok(None);
    };
    let Some(server) = jdtls::inspect(home, configuration) else {
        return Ok(None);
    };
    // Additional bundles can change Equinox's resolution even when every
    // pinned jar remains present. Qualify the complete flat bundle set,
    // not only the core jar's version or a subset of matching files.
    let expected: BTreeSet<_> = profile
        .files
        .iter()
        .filter_map(|pin| pin.path.strip_prefix("plugins").ok())
        .map(Path::to_path_buf)
        .collect();
    let Ok(entries) = fs::read_dir(server.home.join("plugins")) else {
        return Ok(None);
    };
    let actual = entries
        .take(expected.len() + 1)
        .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
        .collect::<Result<BTreeSet<_>, _>>();
    if actual.as_ref().ok() != Some(&expected) {
        return Ok(None);
    }
    let Some((_, config)) = profile
        .configurations
        .iter()
        .find(|(name, _)| name == configuration)
    else {
        return Ok(None);
    };
    for pin in std::iter::once(config).chain(&profile.files) {
        check_cancelled()?;
        if !matches_file(&server.home, pin) {
            return Ok(None);
        }
    }
    Ok(Some(server))
}

/// Validates the exact read-only JDT LS files used by the packaged extensions.
pub fn inspect_server(
    home: &Path,
    os: &str,
    arch: &str,
    check_cancelled: &dyn Fn() -> Result<(), FunctionError>,
) -> Result<Option<ServerInstallation>, FunctionError> {
    inspect_server_with(home, os, arch, &ServerProfile::pinned()?, check_cancelled)
}

fn resolve_with(
    params: &Value,
    environment: &SearchEnvironment,
    profile: &ServerProfile,
    check_cancelled: &dyn Fn() -> Result<(), FunctionError>,
) -> Result<Value, FunctionError> {
    check_cancelled()?;
    let os = params["platform"]["os"].as_str().ok_or_else(invalid)?;
    let arch = params["platform"]["arch"].as_str().ok_or_else(invalid)?;
    if !["linux", "macos", "windows"].contains(&os) || !["x86_64", "arm64"].contains(&arch) {
        return Err(invalid().into());
    }
    let declarations = params["dependencies"].as_array().ok_or_else(invalid)?;
    let mut declared = BTreeSet::new();
    for dependency in declarations {
        let id = dependency["id"].as_str().ok_or_else(invalid)?;
        let expected = match id {
            JDK_ID => JDK_VERSION,
            JDTLS_ID => &profile.version,
            _ => return Err(invalid().into()),
        };
        if !declared.insert(id) || dependency["version"].as_str() != Some(expected) {
            return Err(invalid().into());
        }
    }
    if declared != BTreeSet::from([JDK_ID, JDTLS_ID]) {
        return Err(invalid().into());
    }

    let mut runtimes = Vec::new();
    let mut missing = Vec::new();
    // The host has already applied explicit tool selection and probed it.
    // It returns a terminal error for a broken explicit selection instead
    // of asking this hook to silently replace that selection.
    let tool = &params["tools"][SERVER_RUNTIME_TOOL];
    let selected = absolute(tool.get("home"))
        .filter(|_| tool["version"].as_str().is_some_and(java_25))
        .and_then(|home| inspect_jdk(&home, os, arch));
    let explicit_tool = tool["provider"] == "host:default" || tool["layer"] == "pinned";
    if explicit_tool && selected.is_none() {
        return Err(PluginError::new(
            JDK_INVALID,
            "The selected JDK for JDT LS must be a complete Java 25 JDK for this platform. \
Choose a valid JDK or clear its toolchain override to use the managed runtime.",
        )
        .into());
    }
    let jdk = if explicit_tool {
        selected
    } else {
        // A managed installation remains stable when automatic discovery
        // later finds a different JDK. Only an explicit user choice outranks it.
        absolute(params["candidates"].get(JDK_ID))
            .and_then(|home| inspect_jdk(&home, os, arch))
            .or(selected)
    };
    match jdk {
        Some((home, version)) => runtimes.push(json!({"id":JDK_ID,"home":home,"version":version})),
        None => missing.push(JDK_ID),
    }

    let configured = match params["settings"].get(SERVER_HOME_SETTING) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(value) => Some(absolute(Some(value)).ok_or_else(invalid)?),
    };
    let server = if let Some(home) = configured {
        Some(
            inspect_server_with(&home, os, arch, profile, check_cancelled)?
                .ok_or_else(incompatible_server)?,
        )
    } else {
        // A verified cache keeps subsequent starts independent of changes
        // to another editor or package manager's installation.
        let candidates = absolute(params["candidates"].get(JDTLS_ID))
            .into_iter()
            .chain(
                jdtls::candidates(environment, os)
                    .into_iter()
                    .take(MAX_DISCOVERED_SERVERS),
            );
        let mut found = None;
        let mut seen = BTreeSet::new();
        for home in candidates {
            check_cancelled()?;
            if !seen.insert(home.clone()) {
                continue;
            }
            if let Some(server) = inspect_server_with(&home, os, arch, profile, check_cancelled)? {
                found = Some(server);
                break;
            }
        }
        found
    };
    match server {
        Some(server) => runtimes.push(json!({
            "id":JDTLS_ID,"home":server.home,"version":profile.version
        })),
        None => missing.push(JDTLS_ID),
    }
    check_cancelled()?;
    Ok(json!({"runtimes":runtimes,"missing":missing}))
}

/// SDK entry point. Expected absence is a successful plan with `missing` IDs;
/// invalid explicit configuration is an SDK-reported error. Cancellation is
/// checked between each bounded file read and never becomes a failure log.
pub fn resolve_runtimes_function(params: Value, context: &Context) -> Result<Value, FunctionError> {
    localize_result(
        (|| {
            resolve_with(
                &params,
                &SearchEnvironment::current(),
                &ServerProfile::pinned()?,
                &|| context.check_cancelled().map_err(Into::into),
            )
        })(),
        params["language"].as_str().unwrap_or_default(),
    )
}

fn localize_result(
    result: Result<Value, FunctionError>,
    language: &str,
) -> Result<Value, FunctionError> {
    if !language.to_ascii_lowercase().starts_with("zh") {
        return result;
    }
    result.map_err(|failure| {
        let FunctionError::Plugin(error) = &failure else {
            return failure;
        };
        let message = match error.code() {
            INVALID_PARAMS => "Java 支持收到的运行环境准备参数无效。",
            SERVER_INCOMPATIBLE => "所选语言服务器与已验证的 JDT LS 1.61.0 文件不一致。请选择该版本，或清空“语言服务器目录”，由 Lithe 自动准备。",
            JDK_INVALID => "JDT LS 所选运行环境必须是适合当前平台的完整 JDK 25。请选择有效的 JDK，或清空该工具链的覆盖设置以使用自动管理的运行环境。",
            NOT_PREPARED => "Java 语言服务环境尚未就绪，请重试 Java 支持的运行环境准备。",
            PROFILE_INVALID => "Java 支持无法读取运行环境要求，请重新安装插件。",
            _ => return failure,
        };
        PluginError::new(error.code(), message).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn runtime_errors_follow_ui_language_and_preserve_categories() {
        for code in ERROR_CODES {
            for language in ["zh-CN", "zh-Hans", "ZH-tw"] {
                let result = localize_result(
                    Err(PluginError::new(*code, "English recovery guidance.").into()),
                    language,
                );
                let Err(FunctionError::Plugin(error)) = result else {
                    panic!("expected a localized plugin error");
                };
                assert_eq!(error.code(), *code);
                assert!(!error.message().is_ascii());
            }
            for language in ["", "en", "en-US", "de"] {
                let result = localize_result(
                    Err(PluginError::new(*code, "English recovery guidance.").into()),
                    language,
                );
                let Err(FunctionError::Plugin(error)) = result else {
                    panic!("expected an English plugin error");
                };
                assert_eq!(error.code(), *code);
                assert_eq!(error.message(), "English recovery guidance.");
            }
        }
        assert!(matches!(
            localize_result(Err(FunctionError::Cancelled), "zh-Hans"),
            Err(FunctionError::Cancelled)
        ));
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "lithe-managed-java-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(dunce::canonicalize(root).unwrap())
        }

        fn jdk(&self, name: &str, os: &str, arch: &str, version: &str) -> PathBuf {
            let home = self.0.join(name);
            for directory in ["bin", "lib", "jmods"] {
                fs::create_dir_all(home.join(directory)).unwrap();
            }
            let os_name = match os {
                "macos" => "Darwin",
                "windows" => "Windows",
                _ => "Linux",
            };
            fs::write(
                home.join("release"),
                format!("JAVA_VERSION=\"{version}\"\nOS_ARCH=\"{arch}\"\nOS_NAME=\"{os_name}\"\n"),
            )
            .unwrap();
            let mut bytes = vec![0u8; 8192];
            match os {
                "linux" => {
                    bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
                    let machine: u16 = if arch == "arm64" { 183 } else { 62 };
                    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
                }
                "macos" => {
                    bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
                    let cpu: u32 = if arch == "arm64" {
                        0x0100000c
                    } else {
                        0x01000007
                    };
                    bytes[4..8].copy_from_slice(&cpu.to_le_bytes());
                }
                "windows" => {
                    bytes[..2].copy_from_slice(b"MZ");
                    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
                    bytes[128..132].copy_from_slice(b"PE\0\0");
                    let machine: u16 = if arch == "arm64" { 0xaa64 } else { 0x8664 };
                    bytes[132..134].copy_from_slice(&machine.to_le_bytes());
                }
                _ => unreachable!(),
            }
            for name in ["java", "javac"] {
                let suffix = if os == "windows" { ".exe" } else { "" };
                let path = home.join(format!("bin/{name}{suffix}"));
                fs::write(&path, &bytes).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
            fs::write(home.join("lib/modules"), b"module image").unwrap();
            fs::write(home.join("jmods/java.base.jmod"), b"base module").unwrap();
            home
        }

        fn server(&self, name: &str) -> (PathBuf, ServerProfile) {
            let home = self.0.join(name);
            fs::create_dir_all(home.join("plugins")).unwrap();
            fs::create_dir_all(home.join("config_linux")).unwrap();
            let paths = [
                "plugins/org.eclipse.equinox.launcher_1.8.0.fixture.jar",
                "plugins/org.eclipse.jdt.ls.core_1.61.0.fixture.jar",
                "config_linux/config.ini",
            ];
            let mut files = Vec::new();
            for path in paths {
                let content = format!("verified fixture {path}");
                fs::write(home.join(path), &content).unwrap();
                files.push(PinnedFile {
                    path: PathBuf::from(path),
                    size: content.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
                });
            }
            let config = files.pop().unwrap();
            (
                home,
                ServerProfile {
                    version: "1.61.0".into(),
                    files,
                    configurations: vec![("config_linux".into(), config)],
                },
            )
        }

        fn params(&self) -> Value {
            json!({
                "platform":{"os":"linux","arch":"x86_64"},
                "dependencies":[{"id":"jdk","version":JDK_VERSION},{"id":"jdtls","version":"1.61.0"}],
                "candidates":{},"tools":{},"settings":{}
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn requests_only_the_missing_dependency_and_never_changes_the_project_jdk() {
        let fixture = Fixture::new();
        let (server, profile) = fixture.server("jdtls");
        let jdk = fixture.jdk("jdk", "linux", "x86_64", "25.0.1");
        let environment = SearchEnvironment::default();
        for (has_jdk, has_server, expected) in [
            (false, false, vec![JDK_ID, JDTLS_ID]),
            (true, false, vec![JDTLS_ID]),
            (false, true, vec![JDK_ID]),
            (true, true, vec![]),
        ] {
            let mut params = fixture.params();
            params["tools"]["java"] = json!({"home":"/project/jdk8","version":"8.0.392"});
            if has_jdk {
                params["candidates"][JDK_ID] = json!(jdk);
            }
            if has_server {
                params["candidates"][JDTLS_ID] = json!(server);
            }
            let before = params.clone();
            let result = resolve_with(&params, &environment, &profile, &|| Ok(())).unwrap();
            assert_eq!(result["missing"], json!(expected));
            assert_eq!(
                result["runtimes"].as_array().unwrap().len(),
                2 - expected.len()
            );
            assert_eq!(params, before, "runtime planning is read-only");
            assert!(result.get("tools").is_none(), "no project JDK override");
        }
    }

    #[test]
    fn reuses_the_host_selected_jdk_and_external_verified_server_without_downloads() {
        let fixture = Fixture::new();
        let (server, profile) = fixture.server("external-server");
        let selected = fixture.jdk("selected-jdk", "linux", "x86_64", "25.0.4");
        let cached = fixture.jdk("cached-jdk", "linux", "x86_64", JDK_VERSION);
        let mut params = fixture.params();
        params["tools"][SERVER_RUNTIME_TOOL] = json!({
            "home":selected,"version":"25.0.4","provider":"host:default","layer":"global"
        });
        params["candidates"][JDK_ID] = json!(cached);
        let environment = SearchEnvironment {
            server_home: Some(server.clone()),
            ..SearchEnvironment::default()
        };
        let result = resolve_with(&params, &environment, &profile, &|| Ok(())).unwrap();
        assert_eq!(result["missing"], json!([]));
        assert_eq!(result["runtimes"][0]["home"], json!(selected));
        assert_eq!(result["runtimes"][0]["version"], "25.0.4");
        assert_eq!(result["runtimes"][1]["home"], json!(server));
    }

    #[test]
    fn incomplete_or_mixed_server_is_not_reused_and_an_explicit_one_is_an_error() {
        let fixture = Fixture::new();
        let (server, profile) = fixture.server("server");
        let core = server.join("plugins/org.eclipse.jdt.ls.core_1.61.0.fixture.jar");
        let mut content = fs::read(&core).unwrap();
        content[0] ^= 1; // Same name and size, so existence/version checks cannot catch it.
        fs::write(&core, content).unwrap();
        let mut params = fixture.params();
        params["candidates"][JDTLS_ID] = json!(server);
        let environment = SearchEnvironment::default();
        let result = resolve_with(&params, &environment, &profile, &|| Ok(())).unwrap();
        assert_eq!(result["missing"], json!([JDK_ID, JDTLS_ID]));
        params["settings"][SERVER_HOME_SETTING] = json!(server);
        match resolve_with(&params, &environment, &profile, &|| Ok(())).unwrap_err() {
            FunctionError::Plugin(error) => assert_eq!(error.code(), SERVER_INCOMPATIBLE),
            error => panic!("unexpected error {error:?}"),
        }
        fs::remove_file(core).unwrap();
        assert!(
            inspect_server_with(&server, "linux", "x86_64", &profile, &|| Ok(()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn full_jdk_checks_are_platform_specific_and_reject_jre_wrong_arch_and_partial_files() {
        let fixture = Fixture::new();
        for os in ["linux", "macos", "windows"] {
            for arch in ["x86_64", "arm64"] {
                let home = fixture.jdk(&format!("{os}-{arch}"), os, arch, "25.0.1");
                assert!(inspect_jdk(&home, os, arch).is_some(), "{os}/{arch}");
                let other_arch = if arch == "arm64" { "x86_64" } else { "arm64" };
                assert!(inspect_jdk(&home, os, other_arch).is_none());
                let suffix = if os == "windows" { ".exe" } else { "" };
                fs::write(home.join(format!("bin/java{suffix}")), b"").unwrap();
                assert!(inspect_jdk(&home, os, arch).is_none());
            }
        }
        for version in ["21.0.9", "24.0.2", "26.0.1", "25-ea"] {
            let home = fixture.jdk(version, "linux", "x86_64", version);
            assert!(inspect_jdk(&home, "linux", "x86_64").is_none(), "{version}");
        }
        let jre = fixture.jdk("jre", "linux", "x86_64", "25.0.1");
        fs::remove_file(jre.join("bin/javac")).unwrap();
        assert!(inspect_jdk(&jre, "linux", "x86_64").is_none());
    }

    #[test]
    fn native_launcher_header_must_match_release_metadata_and_executable_permission() {
        let fixture = Fixture::new();
        let home = fixture.jdk("jdk", "linux", "x86_64", "25");
        let java = home.join("bin/java");
        let mut bytes = fs::read(&java).unwrap();
        bytes[18..20].copy_from_slice(&183u16.to_le_bytes());
        fs::write(&java, &bytes).unwrap();
        assert!(inspect_jdk(&home, "linux", "x86_64").is_none());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        fs::write(&java, &bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(java, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(inspect_jdk(&home, "linux", "x86_64").is_none());
        }
    }

    #[test]
    fn an_invalid_explicit_service_jdk_does_not_fall_back_to_the_cache() {
        let fixture = Fixture::new();
        let (_, profile) = fixture.server("server");
        let selected = fixture.jdk("selected-jre", "linux", "x86_64", "25.0.1");
        fs::remove_file(selected.join("bin/javac")).unwrap();
        let cached = fixture.jdk("managed-jdk", "linux", "x86_64", JDK_VERSION);
        let mut params = fixture.params();
        params["tools"][SERVER_RUNTIME_TOOL] = json!({
            "home":selected,"version":"25.0.1","provider":"host:default","layer":"global"
        });
        params["candidates"][JDK_ID] = json!(cached);
        match resolve_with(&params, &SearchEnvironment::default(), &profile, &|| Ok(()))
            .unwrap_err()
        {
            FunctionError::Plugin(error) => assert_eq!(error.code(), JDK_INVALID),
            error => panic!("unexpected error {error:?}"),
        }
    }

    #[test]
    fn verified_cache_precedes_an_automatically_discovered_jdk() {
        let fixture = Fixture::new();
        let (_, profile) = fixture.server("server");
        let discovered = fixture.jdk("automatic-jdk", "linux", "x86_64", "25.0.4");
        let cached = fixture.jdk("managed-jdk", "linux", "x86_64", JDK_VERSION);
        let mut params = fixture.params();
        params["tools"][SERVER_RUNTIME_TOOL] = json!({
            "home":discovered,"version":"25.0.4","provider":"host:discovered","layer":"automatic"
        });
        params["candidates"][JDK_ID] = json!(cached);
        let result =
            resolve_with(&params, &SearchEnvironment::default(), &profile, &|| Ok(())).unwrap();
        assert_eq!(result["runtimes"][0]["home"], json!(cached));
        assert_eq!(result["runtimes"][0]["version"], JDK_VERSION);
    }

    #[test]
    fn an_extra_bundle_is_not_a_verified_distribution() {
        let fixture = Fixture::new();
        let (home, profile) = fixture.server("server");
        fs::write(home.join("plugins/another.bundle_2.0.jar"), b"extra bundle").unwrap();
        assert!(
            inspect_server_with(&home, "linux", "x86_64", &profile, &|| Ok(()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn cancellation_interrupts_qualification_without_a_partial_result() {
        use std::cell::Cell;
        let fixture = Fixture::new();
        let (server, profile) = fixture.server("server");
        let mut params = fixture.params();
        params["candidates"][JDTLS_ID] = json!(server);
        let checks = Cell::new(0);
        let result = resolve_with(&params, &SearchEnvironment::default(), &profile, &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 4 {
                Err(FunctionError::Cancelled)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(FunctionError::Cancelled)));
        assert_eq!(checks.get(), 4);
    }

    #[test]
    fn the_compiled_profile_binds_all_platform_templates_to_the_pinned_archive() {
        let profile = ServerProfile::pinned().unwrap();
        assert_eq!(profile.version, "1.61.0");
        assert!(profile.files.len() > 100);
        for (os, arch) in [
            ("linux", "x86_64"),
            ("linux", "arm64"),
            ("macos", "x86_64"),
            ("macos", "arm64"),
            ("windows", "x86_64"),
            ("windows", "arm64"),
        ] {
            let name = jdtls::configuration_name(os, arch).unwrap();
            assert!(profile.configurations.iter().any(|(key, _)| key == name));
        }
    }
}
