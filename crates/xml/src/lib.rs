//! Compatibility facade for AniCraft's historical XML tree API.
//!
//! Parsing is owned by the shared `craft-xml` crate. This layer preserves AniCraft's public
//! element shape and mixed-content representation for its XFL and SVG document adapters.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;

/// Largest document read (bytes).
pub const MAX_INPUT: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 256;
const MAX_ELEMENTS: usize = 4_000_000;

#[derive(Debug, Default, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<Element>,
    /// The element's direct character data with references resolved.
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn f32(&self, name: &str) -> Option<f32> {
        self.attr(name).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
    }
    pub fn u32(&self, name: &str) -> Option<u32> {
        self.attr(name).and_then(|v| v.trim().parse::<u32>().ok())
    }
    pub fn bool(&self, name: &str) -> Option<bool> {
        self.attr(name).map(|v| v.trim() == "true")
    }
    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
    /// Children of the first child named `name` (XFL wraps lists: `<layers><DOMLayer/>…`).
    pub fn list(&self, name: &str) -> &[Element] {
        self.child(name).map_or(&[], |c| c.children.as_slice())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum XmlError {
    #[error("the XML is too large")]
    TooLarge,
    #[error("the XML is nested too deeply")]
    TooDeep,
    #[error("the XML has too many elements")]
    TooMany,
    #[error("malformed XML at byte {0}: {1}")]
    Syntax(usize, &'static str),
}

/// The name of the pseudo-elements that hold character data in mixed content.
pub const TEXT_NODE: &str = "#text";

fn options() -> craft_xml::Options<'static> {
    // Use craft-xml's documented attribute budgets. Deriving attribute counts from the input
    // byte limit allowed tens of millions of repeated attributes to accumulate before the
    // facade collapsed them into its HashMap, exhausting memory on a small element.
    let default_limits = craft_xml::Limits::DEFAULT;
    craft_xml::Options {
        strict_end_tags: true,
        unknown_entity: craft_xml::UnknownEntity::Literal,
        doctype: craft_xml::Doctype::Ignore,
        limits: craft_xml::Limits {
            max_input: MAX_INPUT,
            max_depth: MAX_DEPTH + 1,
            max_nodes: MAX_ELEMENTS * 2 + 1,
            max_elements: MAX_ELEMENTS,
            max_attrs: default_limits.max_attrs,
            max_attrs_total: default_limits.max_attrs_total,
        },
        ..craft_xml::Options::default()
    }
}

fn map_error(error: craft_xml::Error) -> XmlError {
    match error {
        craft_xml::Error::TooLarge(_) => XmlError::TooLarge,
        craft_xml::Error::TooDeep(_) => XmlError::TooDeep,
        craft_xml::Error::TooManyNodes(_)
        | craft_xml::Error::TooManyElements(_)
        | craft_xml::Error::TooManyAttrs(_)
        | craft_xml::Error::TooManyAttrsTotal(_) => XmlError::TooMany,
        craft_xml::Error::NoRoot => XmlError::Syntax(0, "no root element"),
        craft_xml::Error::Syntax { at, msg } if msg.contains("does not match") => XmlError::Syntax(at, "mismatched end tag"),
        craft_xml::Error::Syntax { at, msg } if msg.contains("not closed") => XmlError::Syntax(at, "missing end tag"),
        craft_xml::Error::Syntax { at, msg } if msg.contains("no start tag") => XmlError::Syntax(at, "unexpected end tag"),
        craft_xml::Error::Syntax { at, .. } => XmlError::Syntax(at, "invalid XML syntax"),
        craft_xml::Error::Encoding(at) => XmlError::Syntax(at, "invalid UTF-8"),
        craft_xml::Error::Doctype => XmlError::Syntax(0, "unexpected DOCTYPE"),
        craft_xml::Error::UnknownEntity { at, .. } => XmlError::Syntax(at, "unknown entity"),
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn text_node(text: &str) -> Element {
    Element { name: TEXT_NODE.to_string(), text: text.to_string(), ..Element::default() }
}

fn charge_item(used: &mut usize) -> Result<(), XmlError> {
    *used = (*used).saturating_add(1);
    if *used > MAX_ELEMENTS { Err(XmlError::TooMany) } else { Ok(()) }
}

fn convert(source: &craft_xml::El, mixed: &[&str], used: &mut usize) -> Result<Element, XmlError> {
    use craft_xml::Node;

    charge_item(used)?;
    let is_mixed = mixed.contains(&local(&source.name));
    let mut element = Element { name: source.name.clone(), ..Element::default() };
    for (name, value) in &source.attrs {
        element.attrs.insert(name.clone(), value.clone());
    }
    let has_elements = source.children.iter().any(|node| matches!(node, Node::El(_)));
    for node in &source.children {
        match node {
            Node::El(child) => element.children.push(convert(child, mixed, used)?),
            Node::Text(text) | Node::CData(text) => {
                element.text.push_str(text);
                if is_mixed {
                    charge_item(used)?;
                    element.children.push(text_node(text));
                }
            }
            Node::Pi { .. } | Node::Comment(_) => {}
        }
    }
    if !is_mixed && has_elements && element.text.trim().is_empty() {
        element.text.clear();
    }
    Ok(element)
}

/// Parses a document and returns its root element.
pub fn parse(src: &[u8]) -> Result<Element, XmlError> {
    let root = options().parse(src).map_err(map_error)?;
    let mut used = 0;
    convert(&root, &[], &mut used)
}

/// Parses a document whose `mixed` elements keep character stretches among their children.
pub fn parse_mixed(src: &[u8], mixed: &[&str]) -> Result<Element, XmlError> {
    let root = options().parse(src).map_err(map_error)?;
    let mut used = 0;
    convert(&root, mixed, &mut used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_entities_ignored_doctypes_and_first_root() {
        let root = parse(b"<!DOCTYPE a [<!ENTITY ignored 'x'>]><a value='&unknown;'>&bad;&amp c</a><other/>").unwrap();
        assert_eq!(root.attr("value"), Some("&unknown;"));
        assert_eq!(root.text, "&bad;&amp c");
    }

    #[test]
    fn mixed_text_stays_ordered_and_plain_indentation_stays_dropped() {
        let root = parse_mixed(b"<svg><text>Hello <tspan>big</tspan> world&amp;<!-- c --> more</text><g>\n  <path/>\n</g></svg>", &["text", "tspan"])
            .unwrap();
        let text = &root.children[0];
        let parts: Vec<(&str, &str)> = text.children.iter().map(|child| (child.name.as_str(), child.text.as_str())).collect();
        assert_eq!(parts, [(TEXT_NODE, "Hello "), ("tspan", "big"), (TEXT_NODE, " world& more")]);
        assert_eq!(root.children[1].text, "");
    }

    #[test]
    fn malformed_nesting_and_old_resource_limits_remain_errors() {
        for malformed in [b"<a><b></a>".as_slice(), b"<a><b/>".as_slice()] {
            assert!(parse(malformed).is_err());
        }
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 2), "</a>".repeat(MAX_DEPTH + 2));
        assert!(matches!(parse(deep.as_bytes()), Err(XmlError::TooDeep)));
    }

    #[test]
    fn repeated_attributes_cannot_bypass_the_shared_reader_budget() {
        let mut input = String::from("<root");
        for _ in 0..=craft_xml::Limits::DEFAULT.max_attrs {
            input.push_str(" x=''");
        }
        input.push_str("/>");
        assert!(matches!(parse(input.as_bytes()), Err(XmlError::TooMany)));
    }

    #[test]
    fn duplicate_attributes_keep_the_historical_last_value() {
        let root = parse(b"<root x='first' x='last'/>").unwrap();
        assert_eq!(root.attr("x"), Some("last"));
    }
}
