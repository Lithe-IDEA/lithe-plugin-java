//! Safe editing of the Eclipse WTP Dynamic Web Module facet version.
//! The source-settings host owns conflict checking and publication. This
//! module prepares only an XML replacement and preserves unrelated entries.
use lithe_plugin_sdk::{FunctionError, PluginError, Value};
use serde_json::{json, Map};
use xmltree::{Element, XMLNode};

pub const FILE: &str = ".settings/org.eclipse.wst.common.project.facet.core.xml";
const FIELD: &str = "webFacetVersion";
const SUPPORTED: &[&str] = &["2.5", "3.0", "3.1", "4.0", "5.0", "6.0"];

fn failed() -> FunctionError {
    PluginError::new(
        "java.webFacet.invalid",
        "The Web Facet source is invalid or has changed. Reload the Java project structure.",
    )
    .into()
}

fn parsed(bytes: &[u8]) -> Result<Element, FunctionError> {
    if bytes.windows(9).any(|window| window.eq_ignore_ascii_case(b"<!DOCTYPE")) {
        return Err(failed());
    }
    let document = Element::parse(bytes).map_err(|_| failed())?;
    if document.name != "faceted-project" {
        return Err(failed());
    }
    Ok(document)
}

fn web_version(document: &Element) -> Result<Option<String>, FunctionError> {
    let mut version = None;
    for element in document.children.iter().filter_map(XMLNode::as_element) {
        if element.name != "installed"
            || element.attributes.get("facet").map(String::as_str) != Some("jst.web")
        {
            continue;
        }
        if version.is_some() {
            return Err(failed());
        }
        let found = element.attributes.get("version").ok_or_else(failed)?;
        if found.is_empty() || found.len() > 16 {
            return Err(failed());
        }
        version = Some(found.clone());
    }
    Ok(version)
}

/// Returns the fields of one existing Eclipse Dynamic Web Module facet.
/// If a project has no jst.web facet, it gets no editable Web Facet source.
pub fn fields(zh: bool, bytes: &[u8]) -> Result<Option<Vec<Value>>, FunctionError> {
    let document = parsed(bytes)?;
    let Some(version) = web_version(&document)? else {
        return Ok(None);
    };
    let mut options: Vec<Value> = SUPPORTED
        .iter()
        .map(|value| json!({"value": value, "title": value}))
        .collect();
    if !SUPPORTED.contains(&version.as_str()) {
        options.push(json!({"value": version, "title": version}));
    }
    Ok(Some(vec![json!({
        "id": FIELD,
        "title": if zh { "Dynamic Web Module 版本" } else { "Dynamic Web Module version" },
        "kind": "enum",
        "section": "Facet",
        "value": version,
        "effectiveValue": version,
        "effectiveSource": if zh { "Eclipse WTP 工程文件" } else { "Eclipse WTP project file" },
        "options": options,
        "description": if zh {
            "仅修改现有 Eclipse jst.web Facet 的版本。不会重写 .classpath、Web 根或其他 Facet。"
        } else {
            "Changes only the version of an existing Eclipse jst.web facet. Classpaths, Web roots and other facets are not edited."
        }
    })]))
}

/// Applies only the approved version field. The caller owns the source
/// revision and writes this output into a host-staged temporary file.
pub fn edit(bytes: &[u8], values: &Map<String, Value>) -> Result<Vec<u8>, FunctionError> {
    if values.len() != 1 || !values.contains_key(FIELD) {
        return Err(failed());
    }
    let chosen = values[FIELD].as_str().ok_or_else(failed)?;
    let mut document = parsed(bytes)?;
    let original = web_version(&document)?.ok_or_else(failed)?;
    if chosen == original {
        return Ok(bytes.to_vec());
    }
    if !SUPPORTED.contains(&chosen) {
        return Err(failed());
    }
    let mut changed = false;
    for node in &mut document.children {
        if let XMLNode::Element(element) = node {
            if element.name == "installed"
                && element.attributes.get("facet").map(String::as_str) == Some("jst.web")
            {
                element.attributes.insert("version".to_owned(), chosen.to_owned());
                changed = true;
                break;
            }
        }
    }
    if !changed {
        return Err(failed());
    }
    crate::project_settings::write_classpath(bytes, &document)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<faceted-project>
  <!-- preserve -->
  <fixed facet="jst.java"/>
  <installed facet="jst.web" version="3.1"/>
  <installed facet="jst.java" version="17"/>
</faceted-project>
"#;

    #[test]
    fn reads_a_web_facet_and_preserves_other_facets_on_edit() {
        let fields = fields(true, DOCUMENT).unwrap().unwrap();
        assert_eq!(fields[0]["value"], "3.1");
        assert_eq!(fields[0]["id"], FIELD);
        let change = json!({"webFacetVersion": "4.0"});
        let modified = edit(DOCUMENT, change.as_object().unwrap()).unwrap();
        let output = String::from_utf8(modified).unwrap();
        assert!(output.contains("jst.web"));
        assert!(output.contains("version=\"4.0\""));
        assert!(output.contains("jst.java"));
        assert!(output.contains("<!-- preserve -->"));
    }

    #[test]
    fn unchanged_versions_preserve_bytes_exactly() {
        let change = json!({"webFacetVersion": "3.1"});
        assert_eq!(edit(DOCUMENT, change.as_object().unwrap()).unwrap(), DOCUMENT);
    }

    #[test]
    fn rejects_unexpected_edits_and_conflicting_web_facets() {
        for value in [json!({"webFacetVersion": "7.5"}), json!({"webFacetVersion": null}), json!({"other": "4.0"})] {
            assert!(edit(DOCUMENT, value.as_object().unwrap()).is_err());
        }
        let duplicate = br#"<faceted-project>
          <installed facet="jst.web" version="3.0"/>
          <installed facet="jst.web" version="4.0"/>
        </faceted-project>"#;
        assert!(fields(false, duplicate).is_err());
    }

    #[test]
    fn non_web_facets_are_not_editable() {
        let plain = br#"<faceted-project><installed facet="jst.java" version="17"/></faceted-project>"#;
        assert!(fields(false, plain).unwrap().is_none());
    }

    #[test]
    fn refuses_doctype_and_external_entities() {
        let dangerous = br#"<!DOCTYPE faceted-project [<!ENTITY external SYSTEM "file:///etc/passwd">]>
<faceted-project><installed facet="jst.web" version="4.0"/></faceted-project>"#;
        assert!(fields(false, dangerous).is_err());
    }
}
