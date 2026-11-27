//! Small helpers over `xmltree` for the model descriptors. Names are matched
//! by local name, so namespaced and plain descriptors read the same. The
//! parser never fetches a DTD or external entity.

use xmltree::{Element, XMLNode};

/// The document element of `bytes`, or None when it is not well formed.
pub fn parse(bytes: &[u8]) -> Option<Element> {
    Element::parse(bytes).ok()
}

/// The element children of `element` named `name`.
pub fn children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    element
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(move |child| child.name == name)
}

/// Every descendant of `element` named `name`, depth first in document order.
pub fn descendants<'a>(element: &'a Element, name: &str) -> Vec<&'a Element> {
    let mut found = Vec::new();
    let mut stack: Vec<&Element> = vec![element];
    while let Some(current) = stack.pop() {
        if current.name == name && !std::ptr::eq(current, element) {
            found.push(current);
        }
        let mut nested: Vec<&Element> = current
            .children
            .iter()
            .filter_map(XMLNode::as_element)
            .collect();
        nested.reverse();
        stack.extend(nested);
    }
    found
}

/// The value of attribute `name`.
pub fn attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element.attributes.get(name).map(String::as_str)
}
