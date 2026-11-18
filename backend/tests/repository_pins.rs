//! Offline checks of what the repository pins: the plugin SDK the backend
//! links, the plugin manifest and its packaged files, and the third-party
//! inputs the package scripts download.
//!
//! These checks only read files in the repository, so they run on every
//! platform in `cargo test`; the assembled package is checked separately by
//! `scripts/verify-package.sh`.

use lithe_java_backend::jdtls::SERVER_HOME_SETTING;
use lithe_java_backend::launch::{
    EXTENSIONS_DIRECTORY, LAUNCH_FUNCTION_ID, MINIMUM_SERVER_JAVA_VERSION, PACKAGE_LAYOUT,
    SERVER_RUNTIME_TOOL,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const PLUGIN_ID: &str = "io.github.lithe-idea.java-support";

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend/ has a parent")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repository_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn read_json(relative: &str) -> Value {
    serde_json::from_str(&read(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {value}"))
}

#[test]
fn cargo_resolves_the_sdk_commit_recorded_in_the_lock() {
    let lock = read_json("sdk.lock.json");
    let repository = text(&lock, "repository");
    let revision = text(&lock, "commit");
    let commit = text(&lock, "commit");
    assert_eq!(commit.len(), 40, "the lock must name a full commit");

    let manifest = read("backend/Cargo.toml");
    let dependency =
        format!(r#"lithe-plugin-sdk = {{ git = "{repository}", rev = "{revision}" }}"#);
    assert!(
        manifest.lines().any(|line| line.trim() == dependency),
        "backend/Cargo.toml must depend on `{dependency}`"
    );

    let cargo_lock = read("Cargo.lock");
    let source = format!(r#"source = "git+{repository}?rev={revision}#{commit}""#);
    let package = cargo_lock
        .split("[[package]]")
        .find(|package| package.contains("name = \"lithe-plugin-sdk\""))
        .expect("Cargo.lock resolves lithe-plugin-sdk");
    assert!(
        package.contains(&source),
        "Cargo.lock must resolve {source}, found:{package}"
    );
    assert!(package.contains(&format!("version = \"{}\"", text(&lock, "version"))));
}

#[test]
fn the_manifest_keeps_the_community_identity_and_launches_the_packaged_backend() {
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    assert_eq!(text(&manifest, "id"), PLUGIN_ID);
    assert_eq!(manifest["schemaVersion"], 0);
    // Lithe Store lists only unsigned community packages: the vendor is the
    // organization namespace and the package carries no publisher signature,
    // so nothing may require one or ask the host to check it.
    let vendor = &manifest["vendor"];
    assert_eq!(text(vendor, "id"), "io.github.lithe-idea");
    assert_eq!(text(vendor, "displayName"), "Lithe Community");
    assert_eq!(text(vendor, "signatureRequirement"), "none");
    assert!(
        vendor.get("bundledSignatureRequirement").is_none(),
        "a community package has no bundled signature requirement"
    );
    assert!(
        manifest["requires"].get("packageTrust").is_none(),
        "packageTrust only serves vendor.bundledSignatureRequirement"
    );
    let version = text(&manifest, "version");
    assert!(
        version.split('.').count() == 3
            && version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
        "plugin version must be major.minor.patch: {version}"
    );
    // The host starts the binary this crate builds; scripts/assemble-package.sh
    // copies it to the path the manifest names, so the two must not drift.
    let launch = &manifest["backend"]["launch"];
    assert_eq!(text(launch, "kind"), "packaged");
    assert_eq!(
        text(launch, "path"),
        concat!("bin/", env!("CARGO_PKG_NAME")),
        "backend.launch.path must name the lithe-java-backend binary in bin/"
    );
}

#[test]
fn every_file_the_manifest_ships_from_the_repository_exists() {
    let plugin = repository_root().join(PLUGIN_ID);
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    let mut checked = 0;
    for generator in manifest["projectGenerators"].as_array().unwrap() {
        for key in ["path", "darkPath"] {
            let icon = text(&generator["icon"], key);
            assert!(plugin.join(icon).is_file(), "missing icon {icon}");
            checked += 1;
        }
        for step in generator["steps"].as_array().unwrap() {
            if let Some(source) = step["source"].as_str() {
                assert!(
                    plugin.join(source).is_dir(),
                    "missing template directory {source}"
                );
                checked += 1;
            }
        }
    }
    // Java (two icons, three plain templates) and Spring Boot (two icons);
    // the Maven and Gradle templates belong to their build plugins.
    assert!(
        checked >= 7,
        "expected icons and templates, checked {checked}"
    );
}

#[test]
fn language_server_launch_is_resolved_by_the_backend() {
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    let server = &manifest["languageServers"][0];
    let launch = &server["launch"];
    assert_eq!(text(launch, "kind"), "backend");
    assert_eq!(text(launch, "function"), LAUNCH_FUNCTION_ID);
    assert!(
        launch["timeoutMilliseconds"]
            .as_u64()
            .is_some_and(|value| value > 0),
        "a backend launch must bound how long the host waits for the function"
    );
    // The contract reserves these for declarative launches; the backend's
    // answer carries their values instead.
    for forbidden in [
        "arguments",
        "validationArguments",
        "runtimeDependencies",
        "initializationOptions",
        "hostIntegration",
    ] {
        assert!(
            server.get(forbidden).is_none(),
            "languageServers[0].{forbidden} must not accompany a backend launch"
        );
    }
    let requires = &manifest["requires"];
    assert!(requires.get("serverProcess").is_none());
    assert!(requires.get("executableResolution").is_none());
    assert_eq!(requires["backend"], 0);
    assert_eq!(manifest["pluginAPIVersion"], 0);
}

#[test]
fn package_scripts_name_every_path_the_backend_resolves() {
    // plugin.json does not list the packaged JDT LS extensions, so the
    // scripts that assemble and verify the package are held to the backend's
    // layout.
    for script in ["scripts/assemble-package.sh", "scripts/verify-package.sh"] {
        let source = read(script);
        for path in PACKAGE_LAYOUT {
            assert!(
                path.starts_with(&format!("{EXTENSIONS_DIRECTORY}/")),
                "{path} must stay inside {EXTENSIONS_DIRECTORY}/"
            );
            assert!(
                !path.split('/').any(|part| part == ".."),
                "{path} must not leave the package"
            );
            assert!(source.contains(path), "{script} does not check {path}");
        }
    }
}

#[test]
fn the_users_jdt_ls_runs_on_the_declared_runtime_tool() {
    // JDT LS and its JDK come from the user's machine: the package ships
    // neither, the backend answers the runtime tool the manifest defines, and
    // the user can choose the JDT LS folder in a setting the backend reads.
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    let tools: Vec<&str> = manifest["requiresTools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.as_str().unwrap())
        .collect();
    assert_eq!(tools, ["java", SERVER_RUNTIME_TOOL]);
    let definitions = manifest["toolDefinitions"].as_array().unwrap();
    let runtime = definitions
        .iter()
        .find(|definition| definition["id"] == SERVER_RUNTIME_TOOL)
        .expect("the manifest defines the JDT LS runtime tool");
    let java = definitions
        .iter()
        .find(|definition| definition["id"] == "java")
        .unwrap();
    // The runtime finds JDKs where the project JDK is found, plus the JREs
    // bundled inside VS Code and Cursor `redhat.java` extensions.
    let java_discovery = java["discovery"].as_array().unwrap();
    let runtime_discovery = runtime["discovery"].as_array().unwrap();
    assert!(
        runtime_discovery.len() <= 16,
        "Core allows at most 16 discovery entries per tool"
    );
    assert_eq!(&runtime_discovery[..java_discovery.len()], java_discovery);
    let extra_patterns: Vec<&str> = runtime_discovery[java_discovery.len()..]
        .iter()
        .map(|entry| text(entry, "pattern"))
        .collect();
    assert_eq!(
        extra_patterns,
        [
            ".vscode/extensions/redhat.java-*/jre/*",
            ".cursor/extensions/redhat.java-*/jre/*",
        ]
    );
    assert_eq!(runtime["executableLayouts"], java["executableLayouts"]);
    let runtime_pattern = text(&runtime["version"], "pattern");
    for rejected in [
        r#"java version "1.8.0_301""#,
        r#"openjdk version "11.0.20""#,
        r#"openjdk version "17.0.10""#,
        r#"openjdk version "20.0.2""#,
    ] {
        assert!(
            !rejected.contains("version \"21") && {
                // `jdtls-runtime` only matches Java 21+, so an older `java` on
                // PATH cannot shadow a discovered JDK 21+.
                let digits = rejected.split('"').nth(1).unwrap();
                let major: u64 = digits
                    .strip_prefix("1.")
                    .unwrap_or(digits)
                    .split('.')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                major < MINIMUM_SERVER_JAVA_VERSION
            },
            "{rejected}"
        );
    }
    assert!(
        runtime_pattern.contains("2[1-9]") && !runtime_pattern.contains("(?:1\\.)?"),
        "jdtls-runtime version.pattern must require Java 21+: {runtime_pattern}"
    );
    // It exports nothing, so JDT LS keeps the environment the host gives it.
    assert!(runtime.get("exports").is_none());
    assert!(runtime.get("pathPrepend").is_none());

    let setting = manifest["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|setting| setting["id"] == SERVER_HOME_SETTING)
        .expect("the manifest declares the JDT LS folder setting");
    assert_eq!(text(setting, "type"), "directory");
    assert_eq!(text(setting, "default"), "");

    let pins = read_json("third_party/jdtls/manifest.json");
    assert_eq!(
        pins["minimumJavaVersion"].as_u64(),
        Some(MINIMUM_SERVER_JAVA_VERSION),
        "the tested JDT LS and the backend must agree on its oldest Java"
    );
    assert!(
        !repository_root().join("third_party/jdk").exists(),
        "the package ships no JDK"
    );
}

#[test]
fn settings_and_localizations_fit_within_core_text_limits() {
    // Core's `plugin-model` rejects settings or l10n descriptions longer than
    // `MAX_SETTING_TEXT * 4` (480 Unicode scalar values) and labels longer than
    // `MAX_SETTING_TEXT` (120). Catching this offline prevents shipping a
    // manifest the host cannot admit.
    const MAX_SETTING_TEXT: usize = 120;
    const MAX_DESCRIPTION_TEXT: usize = MAX_SETTING_TEXT * 4;

    let check_text = |value: &str, max: usize, field: &str| {
        let count = value.chars().count();
        assert!(
            !value.trim().is_empty() && count <= max,
            "{field} must be non-empty and at most {max} characters, got {count}"
        );
    };

    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    for setting in manifest["settings"].as_array().unwrap() {
        let id = text(setting, "id");
        check_text(
            text(setting, "title"),
            MAX_SETTING_TEXT,
            &format!("{id}.title"),
        );
        if let Some(description) = setting.get("description").and_then(Value::as_str) {
            check_text(
                description,
                MAX_DESCRIPTION_TEXT,
                &format!("{id}.description"),
            );
        }
        for key in ["placeholder", "unit", "section"] {
            if let Some(value) = setting.get(key).and_then(Value::as_str) {
                check_text(value, MAX_SETTING_TEXT, &format!("{id}.{key}"));
            }
        }
        if let Some(options) = setting.get("options").and_then(Value::as_array) {
            for option in options {
                let option_value = text(option, "value");
                check_text(
                    text(option, "title"),
                    MAX_SETTING_TEXT,
                    &format!("{id}.options.{option_value}"),
                );
            }
        }
    }
    for action in manifest["settingActions"].as_array().unwrap() {
        let id = text(action, "id");
        check_text(
            text(action, "title"),
            MAX_SETTING_TEXT,
            &format!("settingActions.{id}.title"),
        );
        if let Some(description) = action.get("description").and_then(Value::as_str) {
            check_text(
                description,
                MAX_DESCRIPTION_TEXT,
                &format!("settingActions.{id}.description"),
            );
        }
    }

    let l10n_dir = repository_root().join(PLUGIN_ID).join("l10n");
    for entry in fs::read_dir(&l10n_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let file = read_json(&format!("{PLUGIN_ID}/l10n/{name}"));
        if let Some(settings) = file.get("settings").and_then(Value::as_object) {
            for (id, item) in settings {
                if let Some(description) = item.get("description").and_then(Value::as_str) {
                    check_text(
                        description,
                        MAX_DESCRIPTION_TEXT,
                        &format!("{name}: settings.{id}.description"),
                    );
                }
                for key in ["title", "placeholder", "unit"] {
                    if let Some(value) = item.get(key).and_then(Value::as_str) {
                        check_text(
                            value,
                            MAX_SETTING_TEXT,
                            &format!("{name}: settings.{id}.{key}"),
                        );
                    }
                }
                if let Some(options) = item.get("options").and_then(Value::as_object) {
                    for (option_value, option_title) in options {
                        let option_title = option_title.as_str().unwrap_or_else(|| {
                            panic!("{name}: settings.{id}.options.{option_value} must be string")
                        });
                        check_text(
                            option_title,
                            MAX_SETTING_TEXT,
                            &format!("{name}: settings.{id}.options.{option_value}"),
                        );
                    }
                }
            }
        }
        if let Some(actions) = file.get("settingActions").and_then(Value::as_object) {
            for (id, item) in actions {
                if let Some(title) = item.get("title").and_then(Value::as_str) {
                    check_text(
                        title,
                        MAX_SETTING_TEXT,
                        &format!("{name}: settingActions.{id}.title"),
                    );
                }
                if let Some(description) = item.get("description").and_then(Value::as_str) {
                    check_text(
                        description,
                        MAX_DESCRIPTION_TEXT,
                        &format!("{name}: settingActions.{id}.description"),
                    );
                }
            }
        }
    }
}

#[test]
fn third_party_inputs_are_pinned_by_https_url_and_sha256() {
    let relative = "third_party/jdtls/manifest.json";
    let mut pins = 0;
    visit(&read_json(relative), &mut |key, value| {
        let lowered = key.to_ascii_lowercase();
        if lowered.ends_with("sha256") {
            assert_sha256(value, &format!("{relative} {key}"));
            pins += 1;
        } else if lowered.ends_with("url") {
            assert!(value.starts_with("https://"), "{relative} {key}: {value}");
        }
    });
    assert!(pins >= 4, "{relative} pins only {pins} checksums");
}

#[test]
fn every_fixture_is_valid_json() {
    // Some fixtures are contract samples kept for the host APIs that will
    // carry them (see MIGRATION.md); they must at least stay parseable.
    let mut directories = vec![repository_root().join("fixtures")];
    let mut parsed = 0;
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let text = fs::read_to_string(&path).unwrap();
                serde_json::from_str::<Value>(&text)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                parsed += 1;
            }
        }
    }
    assert!(
        parsed >= 18,
        "expected the ported fixtures, parsed {parsed}"
    );
}

fn visit(value: &Value, check: &mut impl FnMut(&str, &str)) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match child {
                    Value::String(text) => check(key, text),
                    other => visit(other, check),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| visit(item, check)),
        _ => {}
    }
}

fn assert_sha256(value: &str, label: &str) {
    assert!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is not a lowercase SHA-256: {value}"
    );
}
