//! Command-level tests of the Java commands, ported from Rust Core's
//! `src/tests/languages.rs`. Maven commands moved to the Maven plugin and
//! `runConfig.generate` stayed in Core, so their tests are not here.

use super::support::{execute_json, temporary_root};
use serde_json::Value;
use std::fs;

#[test]
fn java_launch_command_json_reuses_the_shared_argfile_planner() {
    let classpath = (0..800)
        .map(|index| format!("/workspace/.m2/library-{index}/library-{index}.jar"))
        .collect::<Vec<_>>()
        .join(":");
    let response: Value = serde_json::from_str(&execute_json(
        &serde_json::json!({
            "id": "java-launch-command",
            "command": "execution.planLaunchCommand",
            "payload": {
                "executable": "/Library/Java/JavaVirtualMachines/jdk-21/bin/java",
                "arguments": ["-cp", classpath, "com.example.Main"],
                "argfilePath": "/tmp/lithe-run/launch.argfile",
                "javaFeatureVersion": 21
            }
        })
        .to_string(),
    ))
    .expect("Java launch-command response should be JSON");

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["data"]["kind"], "argfile");
    assert_eq!(
        response["data"]["arguments"],
        serde_json::json!(["@/tmp/lithe-run/launch.argfile", "com.example.Main"])
    );
    assert!(
        response["data"]["argfileContents"]
            .as_str()
            .is_some_and(|contents| contents.starts_with("-cp\n\"/workspace/")),
        "{response}"
    );
}

#[test]
fn jdt_workspace_key_matches_the_shared_compatibility_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/lsp/jdt-workspace-key-v1.json"
    ))
    .expect("JDT workspace-key fixture should be valid JSON");

    for case in fixture["cases"]
        .as_array()
        .expect("JDT workspace-key fixture should contain cases")
    {
        let request = serde_json::json!({
            "id": case["name"],
            "command": "lsp.jdtWorkspaceKey",
            "payload": {
                "workspaceRoot": case["workspaceRoot"],
                "workspaceFingerprint": case["workspaceFingerprint"]
            }
        });
        let response: Value = serde_json::from_str(&execute_json(&request.to_string()))
            .expect("JDT workspace-key response should be JSON");

        assert_eq!(response["ok"], true, "case {}", case["name"]);
        assert_eq!(
            response["data"]["workspaceKey"], case["workspaceKey"],
            "case {}",
            case["name"]
        );
    }
}

#[test]
fn java_workspace_policy_matches_the_shared_compatibility_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/lsp/java-workspace-policy-v1.json"
    ))
    .expect("Java workspace-policy fixture should be valid JSON");

    for case in fixture["cases"]
        .as_array()
        .expect("Java workspace-policy fixture should contain cases")
    {
        let request = serde_json::json!({
            "id": case["name"],
            "command": "java.workspacePolicy",
            "payload": {
                "workspacePaths": case["workspacePaths"],
                "changedPaths": case["changedPaths"]
            }
        });
        let response: Value = serde_json::from_str(&execute_json(&request.to_string()))
            .expect("Java workspace-policy response should be JSON");

        assert_eq!(response["ok"], true, "case {}", case["name"]);
        assert_eq!(response["data"], case["expected"], "case {}", case["name"]);
    }
}

#[test]
fn jdt_workspace_fingerprint_matches_the_shared_compatibility_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/lsp/jdt-workspace-fingerprint-v1.json"
    ))
    .expect("JDT workspace-fingerprint fixture should be valid JSON");

    for case in fixture["cases"]
        .as_array()
        .expect("JDT workspace-fingerprint fixture should contain cases")
    {
        let request = serde_json::json!({
            "id": case["name"],
            "command": "java.jdtWorkspaceFingerprint",
            "payload": {
                "buildFiles": case["buildFiles"],
                "directMavenModules": case["directMavenModules"],
                "gradleWrapperDistribution": case["gradleWrapperDistribution"],
                "jdtlsVersion": case["jdtlsVersion"]
            }
        });
        let response: Value = serde_json::from_str(&execute_json(&request.to_string()))
            .expect("JDT workspace-fingerprint response should be JSON");

        assert_eq!(response["ok"], true, "case {}", case["name"]);
        assert_eq!(
            response["data"]["workspaceFingerprint"], case["workspaceFingerprint"],
            "case {}",
            case["name"]
        );
    }
}

#[test]
fn jdt_cache_retention_matches_the_shared_compatibility_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/lsp/jdt-cache-retention-v1.json"
    ))
    .expect("JDT cache-retention fixture should be valid JSON");

    for case in fixture["cases"]
        .as_array()
        .expect("JDT cache-retention fixture should contain cases")
    {
        let request = serde_json::json!({
            "id": case["name"],
            "command": "java.jdtCacheRetention",
            "payload": {
                "nowUnixSeconds": case["nowUnixSeconds"],
                "activeWorkspaceKey": case["activeWorkspaceKey"],
                "entries": case["entries"]
            }
        });
        let response: Value = serde_json::from_str(&execute_json(&request.to_string()))
            .expect("JDT cache-retention response should be JSON");

        assert_eq!(response["ok"], true, "case {}", case["name"]);
        assert_eq!(response["data"], case["expected"], "case {}", case["name"]);
    }
}

#[test]
fn java_run_markers_match_the_shared_compatibility_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/java/run-markers-v1.json"
    ))
    .expect("Java Run-marker fixture should be valid JSON");
    for case in fixture["cases"]
        .as_array()
        .expect("Java Run-marker fixture should contain cases")
    {
        let response: Value = serde_json::from_str(&execute_json(
            &serde_json::json!({
                "id": case["name"],
                "command": "java.runMarkers",
                "payload": case["request"]
            })
            .to_string(),
        ))
        .expect("Java Run-marker response should be JSON");
        assert_eq!(response["ok"], true, "case {}: {response}", case["name"]);
        assert_eq!(response["data"], case["expected"], "case {}", case["name"]);
    }
}

#[test]
fn java_core_commands_return_shared_runtime_and_structure_data() {
    let root = temporary_root("java");
    fs::create_dir_all(root.join("src/main/java/com/example"))
        .expect("Java source should be creatable");
    fs::write(
        root.join("src/main/java/com/example/App.java"),
        "package com.example;\n@SpringBootApplication\nclass App {\n    static void main(String[] args) {}\n}\n",
    )
    .expect("Java source should be writable");

    let structure = serde_json::json!({
        "id": "java-structure",
        "command": "java.structure",
        "payload": {
            "source": "import a.A;\nimport b.B;\ninterface Service { String call(String value); }\n"
        }
    });
    let structure_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&structure).expect("Java structure request should encode"),
    ))
    .expect("Java structure response should be JSON");
    assert_eq!(structure_response["ok"], true);
    assert_eq!(
        structure_response["data"]["foldRegions"][0]["kind"],
        "imports"
    );
    assert!(structure_response["data"].get("testMethods").is_none());
    assert!(structure_response["data"]
        .get("implementationMarkers")
        .is_none());
    let syntax_highlights = structure_response["data"]["syntaxHighlights"]
        .as_array()
        .expect("Java structure should return syntax highlights");
    assert!(syntax_highlights
        .iter()
        .any(|highlight| highlight["role"] == "keyword"));
    for role in ["functionDeclaration", "parameter", "punctuation", "type"] {
        assert!(
            syntax_highlights
                .iter()
                .any(|highlight| highlight["role"] == role),
            "Java structure should include the {role} role"
        );
    }
    assert!(syntax_highlights.iter().all(|highlight| {
        highlight["utf16Start"].as_u64().is_some()
            && highlight["utf16Length"]
                .as_u64()
                .is_some_and(|length| length > 0)
    }));
    let swift_structure = serde_json::json!({
        "id": "swift-structure",
        "command": "java.structure",
        "payload": {
            "source": "struct Demo {\n    func run() {\n        if ready {\n            work()\n        }\n    }\n}\n"
        }
    });
    let swift_structure_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&swift_structure).expect("Swift structure request should encode"),
    ))
    .expect("Swift structure response should be JSON");
    let swift_folds = swift_structure_response["data"]["foldRegions"]
        .as_array()
        .expect("Swift structure should return fold regions");
    assert!(swift_folds
        .iter()
        .any(|fold| { fold["startLine"] == 0 && fold["endLine"] == 6 && fold["kind"] == "type" }));
    assert!(swift_folds.iter().any(|fold| {
        fold["startLine"] == 1 && fold["endLine"] == 5 && fold["kind"] == "method"
    }));
    let code_vision = serde_json::json!({
        "id": "java-vision",
        "command": "java.codeVision",
        "payload": {
            "root": root,
            "targetPath": "src/main/java/com/example/App.java",
            "paths": ["src/main/java/com/example/App.java"]
        }
    });
    let vision_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&code_vision).expect("code vision request should encode"),
    ))
    .expect("code vision response should be JSON");
    assert_eq!(vision_response["ok"], true);
    assert!(vision_response["data"]["hints"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hint| hint["symbol"] == "App"));
    let class_name = serde_json::json!({
        "id": "java-class",
        "command": "java.className",
        "payload": {"source": "package com.example;\nclass App {}", "simpleName": "App"}
    });
    let class_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&class_name).expect("class name request should encode"),
    ))
    .expect("class name response should be JSON");
    assert_eq!(class_response["data"]["className"], "com.example.App");
    let definition = serde_json::json!({
        "id": "java-definition",
        "command": "java.sourceDefinition",
        "payload": {
            "source": "class App {\n    void run() {}\n}",
            "declarationName": "App",
            "memberName": "run"
        }
    });
    let definition_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&definition).expect("definition request should encode"),
    ))
    .expect("definition response should be JSON");
    assert_eq!(definition_response["data"]["line"], 1);
    let member_lookup = |member: &str, declarations_only: bool| -> Value {
        let request = serde_json::json!({
            "id": "java-member",
            "command": "java.sourceDefinition",
            "payload": {
                "source": "class App {\n    void run() { save(); total = 1; }\n    String 名字, total;\n    void save() {}\n}",
                "declarationName": "App",
                "memberName": member,
                "declarationsOnly": declarations_only
            }
        });
        let response: Value = serde_json::from_str(&execute_json(
            &serde_json::to_string(&request).expect("member request should encode"),
        ))
        .expect("member response should be JSON");
        response["data"].clone()
    };
    // The call and the assignment on line 1 come first but declare nothing.
    assert_eq!(member_lookup("save", true)["line"], 3);
    assert_eq!(member_lookup("total", true)["line"], 2);
    assert_eq!(member_lookup("total", true)["utf16Column"], 15);
    assert!(member_lookup("missing", true).is_null());
    assert_eq!(member_lookup("missing", false)["line"], 0);
    let server_port = serde_json::json!({
        "id": "java-port",
        "command": "java.serverPort",
        "payload": {"content": "server:\n  port: 8080\n", "fileExtension": "yml"}
    });
    let port_response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&server_port).expect("server port request should encode"),
    ))
    .expect("server port response should be JSON");
    assert_eq!(port_response["data"]["port"], 8080);
    fs::remove_dir_all(root).expect("Java fixture should be removable");
}

/// Resolves the member at the first occurrence of `marker` in `source`, the
/// marker being the text up to and including the start of the name.
fn referenced_member_line(source: &str, marker: &str, member: &str) -> Value {
    let offset = source.find(marker).expect("marker should exist") + marker.len() - member.len();
    let before = &source[..offset];
    let line = before.matches('\n').count();
    let column = before[before.rfind('\n').map_or(0, |index| index + 1)..]
        .encode_utf16()
        .count();
    let request = serde_json::json!({
        "id": "java-reference",
        "command": "java.sourceDefinition",
        "payload": {
            "source": source,
            "declarationName": "Main",
            "memberName": member,
            "declarationsOnly": true,
            "reference": { "line": line, "utf16Column": column }
        }
    });
    let response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&request).expect("reference request should encode"),
    ))
    .expect("reference response should be JSON");
    assert_eq!(response["ok"], true, "{response}");
    response["data"]["line"].clone()
}

#[test]
fn java_member_definition_stays_inside_the_named_type() {
    let lookup = |declaration: &str| -> Value {
        let request = serde_json::json!({
            "id": "java-member",
            "command": "java.sourceDefinition",
            "payload": {
                "source": "class Main {\n  class Nested {\n    void save() {}\n  }\n  void save() {}\n}\nclass Other {\n  void only() {}\n}",
                "declarationName": declaration,
                "memberName": "save",
                "declarationsOnly": true
            }
        });
        let response: Value = serde_json::from_str(&execute_json(
            &serde_json::to_string(&request).expect("member request should encode"),
        ))
        .expect("member response should be JSON");
        response["data"]["line"].clone()
    };
    assert_eq!(lookup("Main"), 4);
    assert_eq!(lookup("Nested"), 2);
    assert!(lookup("Other").is_null());
    assert!(lookup("Missing").is_null());
}

#[test]
fn java_member_reference_binds_to_the_type_it_is_used_in() {
    let source = "class Main {\n  class Nested {\n    void save() {}\n    void use() { save(); }\n  }\n  void save() {}\n  void run() { this.save(); }\n}";
    assert_eq!(referenced_member_line(source, "use() { save", "save"), 2);
    assert_eq!(referenced_member_line(source, "this.save", "save"), 5);

    let outer = "class Main {\n  void save() {}\n  class Nested {\n    void use() { save(); this.save(); }\n  }\n}";
    assert_eq!(referenced_member_line(outer, "use() { save", "save"), 1);
    assert!(referenced_member_line(outer, "this.save", "save").is_null());
}

#[test]
fn java_member_reference_keeps_methods_and_fields_apart() {
    let source = "class Main {\n  int save;\n  void save() {}\n  void run() { save(); int x = this.save; }\n}";
    assert_eq!(referenced_member_line(source, "run() { save", "save"), 2);
    assert_eq!(referenced_member_line(source, "this.save", "save"), 1);
}

#[test]
fn java_member_reference_answers_nothing_syntax_cannot_prove() {
    let unknown_receiver =
        "class Main {\n  void save() {}\n  void run(Main other) { other.save(); }\n}";
    assert!(referenced_member_line(unknown_receiver, "other.save", "save").is_null());
    let method_reference = "class Main {\n  void save() {}\n  Runnable run = this::save;\n}";
    assert!(referenced_member_line(method_reference, "this::save", "save").is_null());

    // `Base` may declare `save`, and it would win over `Main.save`.
    let inherited = "class Main {\n  void save() {}\n  class Nested extends Base {\n    void use() { save(); }\n  }\n}";
    assert!(referenced_member_line(inherited, "use() { save", "save").is_null());
    let anonymous = "class Main {\n  void save() {}\n  Object task = new Object() {\n    void use() { save(); }\n  };\n}";
    assert!(referenced_member_line(anonymous, "use() { save", "save").is_null());

    for (shadowed, marker) in [
        ("class Main {\n  int value;\n  int run() { int value = 1; return value; }\n}", "return value"),
        ("class Main {\n  int value;\n  int run(int value) { return value; }\n}", "return value"),
        ("class Main {\n  int value;\n  void run() { for (int value : items) { use(value); } }\n}", "use(value"),
        ("class Main {\n  int value;\n  void run() { try {} catch (Exception value) { use(value); } }\n}", "use(value"),
        ("class Main {\n  int value;\n  Fn run = value -> use(value);\n}", "use(value"),
        ("class Main {\n  int value;\n  Fn run = (value, other) -> use(value);\n}", "use(value"),
        ("class Main {\n  int value;\n  void run(Object o) { if (o instanceof String value) { use(value); } }\n}", "use(value"),
    ] {
        assert!(
            referenced_member_line(shadowed, marker, "value").is_null(),
            "{shadowed}"
        );
    }
    // `this.` skips the local, and a local in a nested type's method does not count.
    let qualified = "class Main {\n  int value;\n  int run(int value) { return this.value; }\n}";
    assert_eq!(referenced_member_line(qualified, "this.value", "value"), 1);
    let plain = "class Main {\n  int value;\n  int run() { return value; }\n}";
    assert_eq!(referenced_member_line(plain, "return value", "value"), 1);
}

#[test]
fn java_member_reference_covers_enum_and_record_scopes() {
    let enumeration = "enum Main {\n  ONE;\n  int total;\n  int run() { return total; }\n}";
    assert_eq!(
        referenced_member_line(enumeration, "return total", "total"),
        2
    );
    // A record component is a field, so the outer field must not answer.
    let record =
        "class Main {\n  int x;\n  record Point(int x) {\n    int run() { return x; }\n  }\n}";
    assert_eq!(referenced_member_line(record, "return x", "x"), 2);
}

#[test]
fn java_member_reference_ignores_a_position_outside_the_source() {
    let source = "class Main {\n  void save() {}\n  void run() { save(); }\n}";
    let request = serde_json::json!({
        "id": "java-reference",
        "command": "java.sourceDefinition",
        "payload": {
            "source": source,
            "declarationName": "Main",
            "memberName": "save",
            "declarationsOnly": true,
            "reference": { "line": 9, "utf16Column": 0 }
        }
    });
    let response: Value = serde_json::from_str(&execute_json(
        &serde_json::to_string(&request).expect("reference request should encode"),
    ))
    .expect("reference response should be JSON");
    assert!(response["data"].is_null());
}

#[test]
fn java_test_debug_launch_matches_the_shared_contract_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/debug/java-test-launch-v1.json"
    ))
    .expect("Java test debug fixture should be valid JSON");

    for case in fixture["cases"]
        .as_array()
        .expect("Java test debug fixture should contain cases")
    {
        let request = serde_json::json!({
            "id": case["name"],
            "command": "debug.javaTestLaunch",
            "payload": case["payload"]
        });
        let response: Value = serde_json::from_str(&execute_json(&request.to_string()))
            .expect("Java test debug response should be JSON");

        assert_eq!(response["ok"], true, "fixture case {}", case["name"]);
        assert_eq!(
            response["data"], case["expected"],
            "fixture case {}",
            case["name"]
        );
    }
}
