//! Small helpers over `roxmltree` for ETG XML files.

use roxmltree::Node;

use crate::DeviceError;

/// Parse an ETG number: decimal, `#x`-prefixed hex (ESI/ENI convention) or `0x` hex.
pub(crate) fn num(text: &str) -> Option<u64> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix("#x").or_else(|| t.strip_prefix("#X")) {
        return u64::from_str_radix(h, 16).ok();
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).ok();
    }
    t.parse::<u64>().ok().or_else(|| t.parse::<i64>().ok().map(|v| v as u64))
}

/// First child element named `name`.
pub(crate) fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

/// All child elements named `name`.
pub(crate) fn children<'a, 'i: 'a>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

/// Trimmed text of child `name`.
pub(crate) fn text<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.text()).map(str::trim).filter(|t| !t.is_empty())
}

/// Number in child `name`.
pub(crate) fn child_num(n: Node<'_, '_>, name: &str) -> Option<u64> {
    text(n, name).and_then(num)
}

/// Number in attribute `name`.
pub(crate) fn attr_num(n: Node<'_, '_>, name: &str) -> Option<u64> {
    n.attribute(name).and_then(num)
}

/// `"1"`/`"true"` attribute.
pub(crate) fn attr_bool(n: Node<'_, '_>, name: &str) -> bool {
    matches!(n.attribute(name).map(str::trim), Some("1" | "true"))
}

/// Parse `text` and check the root element.
pub(crate) fn document<'i>(text: &'i str, root: &str) -> Result<roxmltree::Document<'i>, DeviceError> {
    let opts = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| DeviceError::Xml(e.to_string()))?;
    let name = doc.root_element().tag_name().name();
    if name != root {
        return Err(DeviceError::Format(format!("root element is <{name}>, expected <{root}>")));
    }
    Ok(doc)
}
