//! Java syntax candidates for semantic navigation-marker verification.

use tree_sitter::{Node, Parser};

/// Whether a syntax navigation candidate sits on a type or method declaration.
#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub enum JavaNavigationCandidateKind {
    Type,
    Method,
}

/// One declaration whose semantic parent or implementation relation must be verified by JDT LS.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct JavaNavigationCandidate {
    pub line: i64,
    pub utf16_column: i64,
    pub direction: &'static str,
    pub relation: &'static str,
    pub kind: JavaNavigationCandidateKind,
    /// Zero-based `(line, utf16_column)` positions of supertype identifiers in
    /// `extends` / `implements` clauses for `Type` + `"up"` candidates.
    pub super_positions: Vec<(i64, i64)>,
}

/// Extracts real Java type and method declarations without guessing across statement text.
///
/// Syntax only selects bounded candidates. A candidate never becomes a visible
/// marker until JDT LS confirms at least one target, so incomplete or erroneous
/// source cannot create a false gutter icon by itself.
pub fn navigation_candidates(source: &str) -> Vec<JavaNavigationCandidate> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    collect_candidates(tree.root_node(), source.as_bytes(), &mut candidates);
    candidates.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.utf16_column.cmp(&right.utf16_column))
            .then_with(|| left.direction.cmp(right.direction))
            .then_with(|| left.relation.cmp(right.relation))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    candidates.dedup();
    candidates
}

fn collect_candidates(
    node: Node<'_>,
    source: &[u8],
    candidates: &mut Vec<JavaNavigationCandidate>,
) {
    match node.kind() {
        "interface_declaration"
        | "class_declaration"
        | "enum_declaration"
        | "record_declaration" => {
            collect_type_candidates(node, source, candidates);
        }
        "method_declaration" => {
            collect_method_candidates(node, source, candidates);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_candidates(child, source, candidates);
    }
}

fn collect_type_candidates(
    type_decl: Node<'_>,
    source: &[u8],
    candidates: &mut Vec<JavaNavigationCandidate>,
) {
    let Some(name) = type_decl.child_by_field_name("name") else {
        return;
    };
    let point = name.start_position();
    let line = i64::try_from(point.row).unwrap_or(i64::MAX);
    let col = utf16_column(source, point.row, point.column);

    let modifiers = type_modifiers_text(type_decl, source);
    let is_final = modifiers.split_whitespace().any(|value| value == "final");

    match type_decl.kind() {
        "interface_declaration" => {
            candidates.push(JavaNavigationCandidate {
                line,
                utf16_column: col,
                direction: "down",
                relation: "interface",
                kind: JavaNavigationCandidateKind::Type,
                super_positions: Vec::new(),
            });
            let supers = collect_super_positions(type_decl, "extends_interfaces", source);
            if !supers.is_empty() {
                candidates.push(JavaNavigationCandidate {
                    line,
                    utf16_column: col,
                    direction: "up",
                    relation: "interface",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: supers,
                });
            }
        }
        "class_declaration" => {
            if !is_final {
                candidates.push(JavaNavigationCandidate {
                    line,
                    utf16_column: col,
                    direction: "down",
                    relation: "inheritance",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: Vec::new(),
                });
            }
            let superclass_positions = collect_super_positions(type_decl, "superclass", source);
            if !superclass_positions.is_empty() {
                candidates.push(JavaNavigationCandidate {
                    line,
                    utf16_column: col,
                    direction: "up",
                    relation: "inheritance",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: superclass_positions,
                });
            }
            let interface_positions = collect_super_positions(type_decl, "interfaces", source);
            if !interface_positions.is_empty() {
                candidates.push(JavaNavigationCandidate {
                    line,
                    utf16_column: col,
                    direction: "up",
                    relation: "interface",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: interface_positions,
                });
            }
        }
        "enum_declaration" | "record_declaration" => {
            let interface_positions = collect_super_positions(type_decl, "interfaces", source);
            if !interface_positions.is_empty() {
                candidates.push(JavaNavigationCandidate {
                    line,
                    utf16_column: col,
                    direction: "up",
                    relation: "interface",
                    kind: JavaNavigationCandidateKind::Type,
                    super_positions: interface_positions,
                });
            }
        }
        _ => {}
    }
}

fn collect_super_positions(
    type_decl: Node<'_>,
    clause_kind: &str,
    source: &[u8],
) -> Vec<(i64, i64)> {
    let mut positions = Vec::new();
    let mut cursor = type_decl.walk();
    for child in type_decl.children(&mut cursor) {
        if child.kind() == clause_kind
            || child.kind() == "super_interfaces" && clause_kind == "interfaces"
        {
            extract_type_identifier_positions(child, source, &mut positions);
        }
    }
    if positions.is_empty() {
        if let Some(clause) = type_decl.child_by_field_name(clause_kind) {
            extract_type_identifier_positions(clause, source, &mut positions);
        }
    }
    positions
}

fn extract_type_identifier_positions(clause: Node<'_>, source: &[u8], out: &mut Vec<(i64, i64)>) {
    let mut stack = vec![clause];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "type_identifier" => {
                let point = node.start_position();
                out.push((
                    i64::try_from(point.row).unwrap_or(i64::MAX),
                    utf16_column(source, point.row, point.column),
                ));
            }
            "generic_type" => {
                // Only inspect the base type before `type_arguments`, not generic args.
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "type_arguments" {
                        stack.push(child);
                    }
                }
            }
            "scoped_type_identifier" => {
                // In `pkg.Outer.Inner`, only resolve the rightmost type identifier.
                let mut cursor = node.walk();
                if let Some(last_ident) = node
                    .children(&mut cursor)
                    .filter(|child| child.kind() == "type_identifier")
                    .last()
                {
                    let point = last_ident.start_position();
                    out.push((
                        i64::try_from(point.row).unwrap_or(i64::MAX),
                        utf16_column(source, point.row, point.column),
                    ));
                }
            }
            "type_arguments" => {}
            _ => {
                let mut cursor = node.walk();
                let mut children: Vec<_> = node.children(&mut cursor).collect();
                children.reverse();
                stack.extend(children);
            }
        }
    }
}

fn type_modifiers_text<'a>(type_decl: Node<'_>, source: &'a [u8]) -> &'a str {
    type_decl
        .children(&mut type_decl.walk())
        .find(|child| child.kind() == "modifiers")
        .and_then(|value| value.utf8_text(source).ok())
        .unwrap_or_default()
}

fn collect_method_candidates(
    method: Node<'_>,
    source: &[u8],
    candidates: &mut Vec<JavaNavigationCandidate>,
) {
    let Some(name) = method.child_by_field_name("name") else {
        return;
    };
    let modifiers = method
        .children(&mut method.walk())
        .find(|child| child.kind() == "modifiers");
    let modifier_text = modifiers
        .and_then(|value| value.utf8_text(source).ok())
        .unwrap_or_default();
    let enclosing_type = enclosing_type(method);
    let relation = if enclosing_type.is_some_and(|value| value.kind() == "interface_declaration") {
        "interface"
    } else {
        "inheritance"
    };
    let point = name.start_position();
    let base = JavaNavigationCandidate {
        line: i64::try_from(point.row).unwrap_or(i64::MAX),
        utf16_column: utf16_column(source, point.row, point.column),
        direction: "down",
        relation,
        kind: JavaNavigationCandidateKind::Method,
        super_positions: Vec::new(),
    };

    if is_possible_override_target(modifier_text, enclosing_type, source) {
        candidates.push(base.clone());
    }
    let is_private_or_static = modifier_text
        .split_whitespace()
        .any(|modifier| matches!(modifier, "private" | "static"));
    if !is_private_or_static
        && (has_override_annotation(modifiers, source)
            || enclosing_type.is_some_and(|ty| type_declares_supertypes(ty, source)))
    {
        candidates.push(JavaNavigationCandidate {
            direction: "up",
            relation: super_relation(enclosing_type, source),
            ..base
        });
    }
}

fn type_declares_supertypes(enclosing_type: Node<'_>, source: &[u8]) -> bool {
    let header_end = enclosing_type
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or(enclosing_type.end_byte());
    let header = source
        .get(enclosing_type.start_byte()..header_end)
        .and_then(|value| std::str::from_utf8(value).ok())
        .unwrap_or_default();
    header
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| matches!(word, "extends" | "implements"))
}

fn enclosing_type(mut node: Node<'_>) -> Option<Node<'_>> {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
        ) {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn is_possible_override_target(
    modifier_text: &str,
    enclosing_type: Option<Node<'_>>,
    source: &[u8],
) -> bool {
    if modifier_text
        .split_whitespace()
        .any(|modifier| matches!(modifier, "private" | "static" | "final"))
    {
        return false;
    }
    let Some(enclosing_type) = enclosing_type else {
        return false;
    };
    if enclosing_type.kind() == "interface_declaration" {
        return true;
    }
    let type_modifiers = enclosing_type
        .children(&mut enclosing_type.walk())
        .find(|child| child.kind() == "modifiers")
        .and_then(|value| value.utf8_text(source).ok())
        .unwrap_or_default();
    !type_modifiers
        .split_whitespace()
        .any(|value| value == "final")
}

fn has_override_annotation(modifiers: Option<Node<'_>>, source: &[u8]) -> bool {
    let Some(modifiers) = modifiers else {
        return false;
    };
    let mut cursor = modifiers.walk();
    let found = modifiers.children(&mut cursor).any(|child| {
        matches!(child.kind(), "marker_annotation" | "annotation")
            && child
                .utf8_text(source)
                .is_ok_and(|text| text.trim() == "@Override")
    });
    found
}

fn super_relation(enclosing_type: Option<Node<'_>>, source: &[u8]) -> &'static str {
    let Some(enclosing_type) = enclosing_type else {
        return "inheritance";
    };
    if enclosing_type.kind() == "interface_declaration" {
        return "interface";
    }
    let header_end = enclosing_type
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or(enclosing_type.end_byte());
    let header = source
        .get(enclosing_type.start_byte()..header_end)
        .and_then(|value| std::str::from_utf8(value).ok())
        .unwrap_or_default();
    // JDT LS 1.38 findLinks identifies the parent method but does not report
    // whether its declaring type is an interface. Prefer the interface visual
    // for implementing classes; ordinary subclass overrides remain inheritance.
    // A class that both extends and implements can be ambiguous until JDT LS
    // exposes the declaring-type kind in this response.
    if header
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == "implements")
    {
        "interface"
    } else {
        "inheritance"
    }
}

fn utf16_column(source: &[u8], row: usize, byte_column: usize) -> i64 {
    let line = source
        .split(|value| *value == b'\n')
        .nth(row)
        .unwrap_or_default();
    let prefix = line.get(..byte_column).unwrap_or(line);
    let units = String::from_utf8_lossy(prefix).encode_utf16().count();
    i64::try_from(units).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_declarations_are_candidates_but_constructor_calls_are_not() {
        let source = r#"
interface UploadService {
    UploadResponse createDirectUploadSession(
        UploadRequest request,
        String objectKey
    );
}

class Response {
    Response upload() {
        return new Response(
        );
    }
}
"#;

        let candidates = navigation_candidates(source);

        assert!(candidates.iter().any(|candidate| {
            candidate.line == 1
                && candidate.kind == JavaNavigationCandidateKind::Type
                && candidate.direction == "down"
                && candidate.relation == "interface"
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.line == 2
                && candidate.kind == JavaNavigationCandidateKind::Method
                && candidate.direction == "down"
                && candidate.relation == "interface"
        }));
        // Line 8: class Response (`down`), Line 9: upload() (`down`); `new Response()` is ignored.
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.line >= 8)
                .count(),
            2
        );
    }

    #[test]
    fn type_declarations_and_methods_without_override_annotation_produce_candidates() {
        let source = r#"
interface Extended extends com.example.BaseInterface<String> {}

abstract class AbstractImpl extends BaseClass<Integer> implements Greeter, Marker {
    public String greet(String name) {
        return name;
    }

    private void helper() {}
    static void util() {}
}
"#;

        let candidates = navigation_candidates(source);

        assert!(candidates.iter().any(|candidate| {
            candidate.line == 1
                && candidate.kind == JavaNavigationCandidateKind::Type
                && candidate.direction == "down"
                && candidate.relation == "interface"
        }));
        let iface_up = candidates
            .iter()
            .find(|candidate| {
                candidate.line == 1
                    && candidate.kind == JavaNavigationCandidateKind::Type
                    && candidate.direction == "up"
            })
            .expect("interface extends candidate");
        assert_eq!(iface_up.relation, "interface");
        // Should point at `BaseInterface`, skipping `com.example.` and `<String>`.
        assert_eq!(iface_up.super_positions, vec![(1, 39)]);

        let class_up_inheritance = candidates
            .iter()
            .find(|candidate| {
                candidate.line == 3
                    && candidate.kind == JavaNavigationCandidateKind::Type
                    && candidate.direction == "up"
                    && candidate.relation == "inheritance"
            })
            .expect("class extends candidate");
        assert_eq!(class_up_inheritance.super_positions, vec![(3, 36)]);

        let class_up_interface = candidates
            .iter()
            .find(|candidate| {
                candidate.line == 3
                    && candidate.kind == JavaNavigationCandidateKind::Type
                    && candidate.direction == "up"
                    && candidate.relation == "interface"
            })
            .expect("class implements candidate");
        assert_eq!(class_up_interface.super_positions, vec![(3, 66), (3, 75)]);

        assert!(candidates.iter().any(|candidate| {
            candidate.line == 4
                && candidate.kind == JavaNavigationCandidateKind::Method
                && candidate.direction == "up"
                && candidate.relation == "interface"
        }));
        assert!(!candidates
            .iter()
            .any(|candidate| { matches!(candidate.line, 8 | 9) && candidate.direction == "up" }));
    }

    #[test]
    fn override_methods_get_up_and_possible_down_candidates() {
        let source = r#"
class ServiceImpl implements Service {
    @Override
    public void run() {}
}
"#;

        let candidates = navigation_candidates(source);

        assert!(candidates
            .iter()
            .any(|candidate| { candidate.direction == "up" && candidate.relation == "interface" }));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.direction == "down"));
    }

    #[test]
    fn private_static_final_and_final_class_methods_are_not_down_candidates() {
        let source = r#"
final class Closed {
    void ordinary() {}
    private void hidden() {}
    static void utility() {}
    final void fixed() {}
}
"#;

        assert!(navigation_candidates(source).is_empty());
    }
}
