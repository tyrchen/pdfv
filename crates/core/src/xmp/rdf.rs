//! Packet-local RDF values for XMP property rules.

use std::collections::BTreeSet;

use super::{PDF_D_NS, RDF_NS};
use crate::{BoundedText, Identifier, ParseError, PdfvError, ProfileError, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ArrayKind {
    Bag,
    Seq,
    Alt,
}

impl ArrayKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "bag" => Some(Self::Bag),
            "seq" => Some(Self::Seq),
            "alt" => Some(Self::Alt),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) namespace: BoundedText,
    pub(super) name: Identifier,
    pub(super) lang: Option<BoundedText>,
    pub(super) value: Value,
}

#[derive(Debug)]
pub(super) struct ElementName {
    pub(super) namespace: BoundedText,
    pub(super) name: Identifier,
    pub(super) lang: Option<BoundedText>,
    pub(super) opaque_text: bool,
}

impl Node {
    pub(super) fn matches(&self, namespace: &str, name: &str) -> bool {
        self.namespace.as_str() == namespace && self.name.as_str() == name
    }
}

#[derive(Debug)]
pub(super) enum Value {
    Text(BoundedText),
    OpaqueText,
    Array(ArrayKind, Vec<Node>),
    Structure(Vec<Node>),
    Invalid,
}

impl Value {
    pub(super) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub(super) fn fields(&self) -> &[Node] {
        match self {
            Self::Structure(fields) => fields,
            _ => &[],
        }
    }

    pub(super) fn items(&self) -> &[Node] {
        match self {
            Self::Array(_, items) => items,
            _ => &[],
        }
    }
}

/// XML element's role in the selected RDF tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Root,
    Subject,
    Property,
    Array(ArrayKind),
    Declaration,
}

#[derive(Debug)]
pub(super) struct Frame {
    role: Role,
    fields: Vec<Node>,
    content: Option<Value>,
    resource_literal: Option<BoundedText>,
    resource_fields: bool,
}

impl Frame {
    pub(super) fn new(
        first_root: bool,
        parent: Option<&Self>,
        namespace: &BoundedText,
        name: &Identifier,
    ) -> Result<Option<Self>> {
        let rdf_name = (namespace.as_str() == RDF_NS).then(|| name.as_str());
        let role = if first_root {
            Role::Root
        } else {
            let Some(parent) = parent else {
                return Ok(None);
            };
            match parent.role {
                Role::Root if rdf_name == Some("Description") => Role::Subject,
                Role::Root => return Err(invalid("XMP RDF requires a top-level rdf:Description")),
                Role::Subject
                    if namespace.as_str() == PDF_D_NS && name.as_str() == "declarations" =>
                {
                    Role::Declaration
                }
                Role::Subject if rdf_name != Some("li") => Role::Property,
                Role::Subject => return Err(invalid("Misplaced rdf:li in XMP RDF resource")),
                Role::Array(_) if rdf_name == Some("li") => Role::Property,
                Role::Array(_) => return Err(invalid("XMP RDF array contains a non-list item")),
                Role::Declaration => return Ok(None),
                Role::Property if parent.resource_fields && rdf_name != Some("li") => {
                    Role::Property
                }
                Role::Property => match rdf_name {
                    Some("Bag") => Role::Array(ArrayKind::Bag),
                    Some("Seq") => Role::Array(ArrayKind::Seq),
                    Some("Alt") => Role::Array(ArrayKind::Alt),
                    None | Some("Description") => Role::Subject,
                    Some(_) => {
                        return Err(invalid(
                            "XMP RDF property contains an invalid resource node",
                        ));
                    }
                },
            }
        };
        if namespace.as_str().is_empty() {
            return Err(invalid("XMP RDF element has no namespace"));
        }
        Ok(Some(Self {
            role,
            fields: Vec::new(),
            content: None,
            resource_literal: None,
            resource_fields: false,
        }))
    }

    pub(super) fn attribute(
        &mut self,
        namespace: &BoundedText,
        name: &Identifier,
        value: &str,
        max_text: usize,
    ) -> Result<bool> {
        match (namespace.as_str(), name.as_str(), self.role) {
            (RDF_NS, "parseType", Role::Property) => {
                if value != "Resource" {
                    return Err(invalid("Unsupported RDF parseType in XMP property"));
                }
                self.resource_fields = true;
            }
            (RDF_NS, "resource" | "value", Role::Property) => {
                if self.resource_literal.is_some() {
                    return Err(invalid("XMP RDF property has competing value attributes"));
                }
                self.resource_literal = Some(BoundedText::new(value, max_text)?);
            }
            (_, _, Role::Property | Role::Subject) if super::is_rdf_property(namespace) => {
                self.fields.push(Node {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    lang: None,
                    value: Value::Text(BoundedText::new(value, max_text)?),
                });
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn close(
        self,
        element: ElementName,
        text: String,
        parent: Option<&mut Self>,
        packet: &mut Vec<Node>,
        max_text: usize,
    ) -> Result<bool> {
        if self.role != Role::Property && !text.trim().is_empty() {
            return Err(invalid("Unexpected text in an XMP RDF node"));
        }
        let Some(parent) = parent else {
            return Ok(false);
        };
        match self.role {
            Role::Root => Ok(false),
            Role::Subject if parent.role == Role::Root => {
                packet.extend(self.fields);
                Ok(false)
            }
            Role::Subject => {
                parent.child_value(Value::Structure(self.fields));
                Ok(false)
            }
            Role::Array(kind) => {
                parent.child_value(Value::Array(kind, self.fields));
                Ok(false)
            }
            Role::Property => {
                parent.fields.push(self.property(element, text, max_text)?);
                Ok(true)
            }
            Role::Declaration => {
                parent.fields.push(Node {
                    namespace: element.namespace,
                    name: element.name,
                    lang: element.lang,
                    value: Value::Structure(Vec::new()),
                });
                Ok(true)
            }
        }
    }

    fn child_value(&mut self, value: Value) {
        self.content = Some(if self.content.is_some() {
            Value::Invalid
        } else {
            value
        });
    }

    fn property(self, element: ElementName, text: String, max_text: usize) -> Result<Node> {
        let no_text = text.trim().is_empty();
        let mut value = match (self.content, self.resource_literal) {
            (Some(value), None) if self.fields.is_empty() && no_text => value,
            (None, Some(uri)) if text.is_empty() && !self.resource_fields => Value::Text(uri),
            (None, None) if self.resource_fields || !self.fields.is_empty() => {
                if no_text {
                    Value::Structure(self.fields)
                } else {
                    Value::Invalid
                }
            }
            (None, None) if element.opaque_text => Value::OpaqueText,
            (None, None) => Value::Text(BoundedText::new(text, max_text)?),
            _ => Value::Invalid,
        };
        if matches!(value, Value::Invalid) {
            return Err(invalid("XMP RDF property mixes incompatible value forms"));
        }
        let mut lang = element.lang;
        if let Value::Structure(fields) = &mut value {
            if has_duplicates(fields) {
                return Err(invalid("Duplicate field in XMP RDF resource"));
            }
            if let Some(index) = fields.iter().position(|node| node.matches(RDF_NS, "value")) {
                let node = fields.swap_remove(index);
                value = node.value;
                lang = node.lang.or(lang);
            }
        }
        Ok(Node {
            namespace: element.namespace,
            name: element.name,
            lang,
            value,
        })
    }
}

pub(super) fn has_duplicates(properties: &[Node]) -> bool {
    let mut seen = BTreeSet::new();
    properties
        .iter()
        .any(|node| !seen.insert((node.namespace.as_str(), node.name.as_str())))
}

pub(super) fn invalid(reason: &'static str) -> PdfvError {
    ProfileError::InvalidXml {
        reason: BoundedText::unchecked(reason),
    }
    .into()
}

pub(super) fn charge(count: &mut usize, max: usize) -> Result<()> {
    *count =
        count
            .checked_add(1)
            .filter(|count| *count <= max)
            .ok_or(ParseError::LimitExceeded {
                limit: "max_xmp_properties",
            })?;
    Ok(())
}
