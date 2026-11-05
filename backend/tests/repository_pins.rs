//! Offline checks of what the repository pins: the plugin SDK the backend
//! links, the plugin manifest and its packaged files, and the third-party
//! inputs the package scripts download.
//!
//! These checks only read files in the repository, so they run on every
//! platform in `cargo test`; the assembled package is checked separately by
//! `scripts/verify-package.sh`.

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
    let tag = text(&lock, "tag");
    let commit = text(&lock, "commit");
    assert_eq!(commit.len(), 40, "the lock must name a full commit");
    assert!(text(&lock, "url").starts_with(&format!("{repository}/releases/download/{tag}/")));
    assert_sha256(text(&lock, "sha256"), "sdk.lock.json sha256");

    let manifest = read("backend/Cargo.toml");
    let dependency = format!(r#"lithe-plugin-sdk = {{ git = "{repository}", tag = "{tag}" }}"#);
    assert!(
        manifest.lines().any(|line| line.trim() == dependency),
        "backend/Cargo.toml must depend on `{dependency}`"
    );

    let cargo_lock = read("Cargo.lock");
    let source = format!(r#"source = "git+{repository}?tag={tag}#{commit}""#);
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
fn the_manifest_keeps_the_community_identity_and_declares_no_backend_yet() {
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    assert_eq!(text(&manifest, "id"), PLUGIN_ID);
    assert_eq!(manifest["schemaVersion"], 2);
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
    // The backend is declared once the host exposes the APIs it serves; a
    // declaration before that would start a process nothing calls.
    assert!(
        manifest.get("backend").is_none(),
        "declare the backend together with the host APIs it needs (see MIGRATION.md)"
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
    assert!(
        checked >= 15,
        "expected icons and templates, checked {checked}"
    );
}

#[test]
fn language_server_resources_point_inside_the_assembled_package() {
    let manifest = read_json(&format!("{PLUGIN_ID}/plugin.json"));
    let server = &manifest["languageServers"][0];
    let launch = &server["launch"];
    assert_eq!(text(launch, "packagedRuntime"), "runtime/${arch}/bin/java");
    assert!(text(launch, "entry").starts_with("language-server/plugins/"));
    let resources = &server["hostIntegration"]["resources"];
    let mut paths = vec![
        text(resources, "configuration"),
        text(resources, "lombokAgent"),
        text(resources, "testRunner"),
    ];
    paths.extend(
        resources["extensionBundleDirectories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap()),
    );
    for path in paths {
        assert!(
            path.starts_with("language-server/") && !path.split('/').any(|part| part == ".."),
            "{path} must stay inside language-server/"
        );
    }
}

#[test]
fn third_party_inputs_are_pinned_by_https_url_and_sha256() {
    for relative in [
        "third_party/jdtls/manifest.json",
        "third_party/jdk/manifest.json",
    ] {
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
