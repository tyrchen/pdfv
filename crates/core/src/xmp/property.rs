//! Qualified XMP property definitions and value checks for PDF/A-1 through PDF/A-3.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sxd_xpath_nounsafe::Factory;

use super::{
    PDFA_EXTENSION_NS, PDFA_SCHEMA_NS, PDFA_TYPE_NS,
    rdf::{ArrayKind, Node, Value},
};
use crate::{BoundedText, Identifier, ParseError, Result, ValidationError};

const PROPERTY_NS: &str = "http://www.aiim.org/pdfa/ns/property#";
const FIELD_NS: &str = "http://www.aiim.org/pdfa/ns/field#";

pub(crate) const FAMILY: &str = "xmpProperty";
pub(crate) const PROPERTIES: &[&str] = &[
    "isPredefinedInXMP2004",
    "isPredefinedInXMP2005",
    "isDefinedInMainPackage",
    "isDefinedInCurrentPackage",
    "isValueTypeCorrect",
    "predefinedType",
];

/// The outcome of resolving and validating a property in one PDF/A schema set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XmpPropertyCheck {
    /// The qualified name exists in this predefined schema set.
    pub predefined: bool,
    /// The packet declares a type for the qualified name.
    pub declared: bool,
    /// The property's value matches its selected type.
    pub valid: bool,
    /// Type used for the check, if one could be bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_type: Option<BoundedText>,
}

/// Per-property evidence from a normalized XMP packet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XmpCheckedProperty {
    /// Namespace URI of the qualified property.
    pub namespace_uri: BoundedText,
    /// Local RDF property name.
    pub name: Identifier,
    /// Evidence for PDF/A-1.
    pub pdfa1: XmpPropertyCheck,
    /// Evidence for PDF/A-2 and PDF/A-3.
    pub pdfa23: XmpPropertyCheck,
}

pub(super) fn check(properties: &[Node]) -> Result<Vec<XmpCheckedProperty>> {
    let schemas = Schemas::get()?;
    let declarations = Declarations::read(properties);
    properties
        .iter()
        .filter(|node| !node.matches(PDFA_EXTENSION_NS, "schemas"))
        .map(|node| {
            let namespace = node.namespace.as_str();
            let name = node.name.as_str();
            let open_schema = declarations.schemas.get(namespace);
            Ok(XmpCheckedProperty {
                namespace_uri: node.namespace.clone(),
                name: node.name.clone(),
                pdfa1: SchemaView {
                    predefined: &schemas.pdfa1,
                    extension: open_schema,
                }
                .check(node, name)?,
                pdfa23: SchemaView {
                    predefined: &schemas.pdfa23,
                    extension: open_schema,
                }
                .check(node, name)?,
            })
        })
        .collect()
}

#[derive(Debug, Default)]
struct Schemas {
    pdfa1: Predefined,
    pdfa23: Predefined,
}

impl Schemas {
    fn get() -> Result<&'static Self> {
        static SCHEMAS: LazyLock<Option<Schemas>> = LazyLock::new(Schemas::load);
        SCHEMAS.as_ref().ok_or_else(|| {
            ValidationError::SubsystemUnavailable {
                subsystem: "xmpPredefinedSchemas",
            }
            .into()
        })
    }

    fn load() -> Option<Self> {
        let mut schemas = Self {
            pdfa1: Predefined {
                pdfa1: true,
                ..Predefined::default()
            },
            pdfa23: Predefined::default(),
        };
        for row in data_rows(include_str!("../../data/xmp/properties.tsv")) {
            let [scope, namespace, property, kind] = row.as_slice() else {
                return None;
            };
            schemas.apply(scope, |schema| {
                schema
                    .properties
                    .entry(namespace)
                    .or_default()
                    .insert(property, kind);
            })?;
        }
        for row in data_rows(include_str!("../../data/xmp/structures.tsv")) {
            let [scope, name, namespace, field @ ..] = row.as_slice() else {
                return None;
            };
            let contents = match field {
                [] => None,
                [field, kind] => Some((*field, *kind)),
                _ => return None,
            };
            schemas.apply(scope, |schema| {
                let definition = schema
                    .types
                    .entry(name)
                    .or_insert_with(|| Structure::new(namespace));
                if let Some((field, kind)) = contents {
                    definition.fields.insert(field, kind);
                }
            })?;
        }
        Some(schemas)
    }

    fn apply(&mut self, scope: &str, mut add: impl FnMut(&mut Predefined)) -> Option<()> {
        match scope {
            "common" => {
                add(&mut self.pdfa1);
                add(&mut self.pdfa23);
            }
            "1" => add(&mut self.pdfa1),
            "23" => add(&mut self.pdfa23),
            _ => return None,
        }
        Some(())
    }
}

fn data_rows(data: &str) -> impl Iterator<Item = Vec<&str>> {
    data.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
}

#[derive(Debug, Default)]
struct Predefined {
    pdfa1: bool,
    properties: BTreeMap<&'static str, BTreeMap<&'static str, &'static str>>,
    types: BTreeMap<&'static str, Structure<'static>>,
}

#[derive(Debug)]
struct Structure<'a> {
    namespace: &'a str,
    fields: BTreeMap<&'a str, &'a str>,
}

impl<'a> Structure<'a> {
    fn new(namespace: &'a str) -> Self {
        Self {
            namespace,
            fields: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default)]
struct Declarations<'a> {
    schemas: BTreeMap<&'a str, Extension<'a>>,
}

#[derive(Debug, Default)]
struct Extension<'a> {
    properties: BTreeMap<&'a str, &'a str>,
    types: BTreeMap<String, Structure<'a>>,
}

impl<'a> Declarations<'a> {
    fn read(properties: &'a [Node]) -> Self {
        let mut declarations = Self::default();
        for schema in properties
            .iter()
            .filter(|node| node.matches(PDFA_EXTENSION_NS, "schemas"))
            .flat_map(|container| container.value.items())
        {
            let Some(namespace) = field_text(schema, PDFA_SCHEMA_NS, "namespaceURI") else {
                continue;
            };
            let Some(property_list) = field(schema, PDFA_SCHEMA_NS, "property") else {
                continue;
            };
            let mut extension = Extension::default();
            for value_type in field(schema, PDFA_SCHEMA_NS, "valueType")
                .into_iter()
                .flat_map(Value::items)
            {
                extension.add_type(value_type);
            }
            for property in property_list.items() {
                if let (Some(name), Some(kind)) = (
                    field_text(property, PROPERTY_NS, "name"),
                    field_text(property, PROPERTY_NS, "valueType"),
                ) {
                    extension.properties.entry(name).or_insert(kind);
                }
            }
            declarations.schemas.insert(namespace, extension);
        }
        declarations
    }
}

impl<'a> Extension<'a> {
    fn add_type(&mut self, value_type: &'a Node) {
        let Some(name) = field_text(value_type, PDFA_TYPE_NS, "type") else {
            return;
        };
        let namespace = field_text(value_type, PDFA_TYPE_NS, "namespaceURI");
        let mut structure = Structure::new(namespace.unwrap_or_default());
        for field_node in field(value_type, PDFA_TYPE_NS, "field")
            .into_iter()
            .flat_map(Value::items)
        {
            if let (Some(name), Some(kind)) = (
                field_text(field_node, FIELD_NS, "name"),
                field_text(field_node, FIELD_NS, "valueType"),
            ) {
                structure.fields.insert(name, kind);
            }
        }
        if structure.fields.is_empty() || namespace.is_some() {
            self.types
                .entry(name.to_ascii_lowercase())
                .or_insert(structure);
        }
    }
}

fn field<'a>(node: &'a Node, namespace: &str, name: &str) -> Option<&'a Value> {
    node.value
        .fields()
        .iter()
        .find(|node| node.matches(namespace, name))
        .map(|node| &node.value)
}

fn field_text<'a>(node: &'a Node, namespace: &str, name: &str) -> Option<&'a str> {
    field(node, namespace, name)?.text()
}

#[derive(Debug)]
struct SchemaView<'a> {
    predefined: &'a Predefined,
    extension: Option<&'a Extension<'a>>,
}

impl SchemaView<'_> {
    fn check(&self, node: &Node, name: &str) -> Result<XmpPropertyCheck> {
        let predefined = self
            .predefined
            .properties
            .get(node.namespace.as_str())
            .and_then(|properties| properties.get(name))
            .copied();
        let declared = self
            .extension
            .and_then(|schema| schema.properties.get(name))
            .copied()
            .map(|source| (source, normalize_type(source)))
            .filter(|(_, kind)| self.known_type(kind));
        let (valid, expected) = if let Some((source, kind)) = &declared {
            (self.validate(node, kind)?, Some(*source))
        } else if let Some(kind) = predefined {
            let validator = Self {
                predefined: self.predefined,
                extension: None,
            };
            (validator.validate(node, kind)?, Some(kind))
        } else {
            (false, None)
        };
        Ok(XmpPropertyCheck {
            predefined: predefined.is_some(),
            declared: declared.is_some(),
            valid,
            expected_type: expected.map(reportable_type),
        })
    }

    fn known_type(&self, mut kind: &str) -> bool {
        while let Some((_, child)) = array_type(kind) {
            kind = child;
        }
        (kind != "any" && is_primitive(kind)) || self.structure(kind).is_some()
    }

    fn structure(&self, kind: &str) -> Option<&Structure<'_>> {
        self.extension
            .and_then(|schema| schema.types.get(kind))
            .or_else(|| self.predefined.types.get(kind))
    }

    fn validate(&self, node: &Node, kind: &str) -> Result<bool> {
        if let Some((array, child)) = array_type(kind) {
            return self.validate_array(node, array, child);
        }
        let Some(structure) = self.structure(kind) else {
            return if kind == "lang alt" {
                Ok(valid_language_alternative(node))
            } else if is_primitive(kind) {
                validate_primitive(&node.value, kind, self.predefined.pdfa1)
            } else {
                Ok(false)
            };
        };
        if structure.fields.is_empty() {
            return Ok(node.value.text().is_some());
        }
        let Value::Structure(fields) = &node.value else {
            return Ok(false);
        };
        for field in fields {
            let Some(kind) = structure
                .fields
                .get(field.name.as_str())
                .filter(|_| field.namespace.as_str() == structure.namespace)
            else {
                return Ok(false);
            };
            if !self.validate(field, &normalize_type(kind))? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn validate_array(&self, node: &Node, required: ArrayKind, child: &str) -> Result<bool> {
        let items = match &node.value {
            Value::Array(kind, items) if *kind == required => items.as_slice(),
            _ => return Ok(false),
        };
        for item in items {
            if !self.validate(item, child)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn reportable_type(kind: &str) -> BoundedText {
    if kind.len() <= 128 {
        BoundedText::unchecked(kind)
    } else {
        let available = kind.floor_char_boundary(125);
        BoundedText::unchecked(format!("{}…", kind.get(..available).unwrap_or_default()))
    }
}

pub(crate) fn extension_value_type_defined(
    part: u32,
    value: &str,
    custom: &BTreeSet<String>,
) -> Result<bool> {
    let schemas = Schemas::get()?;
    let predefined = if part == 1 {
        &schemas.pdfa1
    } else {
        &schemas.pdfa23
    };
    let normalized = normalize_type(value);
    let mut base = normalized.as_str();
    while let Some((_, child)) = array_type(base) {
        base = child;
    }
    Ok(custom.contains(base)
        || (base != "any" && is_primitive(base))
        || predefined.types.contains_key(base))
}

fn normalize_type(kind: &str) -> String {
    let mut normalized = Vec::new();
    let lower = kind.to_ascii_lowercase();
    let mut words = lower.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if matches!(word, "open" | "closed") && words.peek() == Some(&"choice") {
            continue;
        }
        if word == "choice" {
            let _ = words.next_if_eq(&"of");
        } else {
            normalized.push(word);
        }
    }
    if normalized.is_empty()
        || (!normalized.ends_with(&["lang", "alt"])
            && normalized
                .last()
                .is_some_and(|word| ArrayKind::parse(word).is_some()))
    {
        normalized.push("text");
    }
    normalized.join(" ")
}

fn array_type(kind: &str) -> Option<(ArrayKind, &str)> {
    let (array, child) = kind.split_once(' ')?;
    Some((ArrayKind::parse(array)?, child))
}

fn valid_language_alternative(node: &Node) -> bool {
    matches!(&node.value, Value::Array(ArrayKind::Alt, items) if items.iter()
        .all(|item| item.value.text().is_some() && item.lang.is_some()))
}

fn is_primitive(kind: &str) -> bool {
    matches!(
        kind,
        "any"
            | "text"
            | "boolean"
            | "integer"
            | "real"
            | "rational"
            | "date"
            | "uri"
            | "url"
            | "xpath"
            | "locale"
            | "mimetype"
            | "propername"
            | "agentname"
            | "gpscoordinate"
            | "renditionclass"
            | "lang alt"
    )
}

fn validate_primitive(value: &Value, kind: &str, pdfa1: bool) -> Result<bool> {
    if kind == "any" {
        return Ok(!matches!(value, Value::Invalid));
    }
    if matches!(value, Value::OpaqueText) {
        return Ok(kind == "text");
    }
    let Some(text) = value.text() else {
        return Ok(false);
    };
    match kind {
        "boolean" => Ok(matches!(text, "True" | "False")),
        "integer" => Ok(signed_digits(text)),
        "real" => Ok(valid_decimal(text)),
        "date" => date_pattern().map(|pattern| text.is_empty() || pattern.is_match(text)),
        "mimetype" => Ok(text
            .split_once('/')
            .is_some_and(|(left, right)| mime_token(left) && mime_token(right))),
        "gpscoordinate" => gps_pattern(pdfa1).map(|pattern| pattern.is_match(text)),
        "xpath" => validate_xpath(text),
        _ => Ok(true),
    }
}

fn signed_digits(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    unsigned.split_once('.').map_or_else(
        || signed_digits(value),
        |(integral, fractional)| {
            !(integral.is_empty() && fractional.is_empty())
                && integral
                    .bytes()
                    .chain(fractional.bytes())
                    .all(|byte| byte.is_ascii_digit())
        },
    )
}

fn mime_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_+.".contains(&byte))
}

fn date_pattern() -> Result<&'static Regex> {
    static DATE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
        r"\A-?[0-9]+(?:-[0-9]+(?:-[0-9]+(?:T[0-9]+(?::[0-9]+(?::[0-9]+(?:\.[0-9]+)?)?)?(?:Z|[+-][0-9]+(?::[0-9]+)?)?)?)?)?\z",
    ).ok()
    });
    DATE.as_ref().ok_or_else(|| {
        ValidationError::SubsystemUnavailable {
            subsystem: "xmpDatePattern",
        }
        .into()
    })
}

fn gps_pattern(pdfa1: bool) -> Result<&'static Regex> {
    static GPS_PDFA1: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"\A[0-9]{2},[0-9]{2}[,.][0-9]{2}[NSEW]\z").ok());
    static GPS: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"\A[0-9]{1,3},[0-9]{1,2}(?:,[0-9]{1,2}|\.[0-9]+)[NSEW]\z").ok()
    });
    let pattern = if pdfa1 { &GPS_PDFA1 } else { &GPS };
    pattern.as_ref().ok_or_else(|| {
        ValidationError::SubsystemUnavailable {
            subsystem: "xmpGpsPattern",
        }
        .into()
    })
}

fn validate_xpath(value: &str) -> Result<bool> {
    // Bound the compiler's recursive-descent grammar and the shape of the resulting AST.
    if value.len() > 1024 {
        return Err(ParseError::LimitExceeded {
            limit: "max_xmp_xpath_bytes",
        }
        .into());
    }
    let mut quote = None;
    let mut operators = 0_u32;
    for byte in value.bytes() {
        if let Some(terminator) = quote {
            if byte == terminator {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if b"([=-+*/<>,|".contains(&byte) {
            operators = operators.saturating_add(1);
        }
        if operators > 64 {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_xpath_operators",
            }
            .into());
        }
    }
    Ok(Factory::new().build(value).is_ok())
}

#[cfg(test)]
mod tests {
    use super::validate_xpath;
    use crate::{ParseError, PdfvError, Result};

    #[test]
    fn test_should_compile_bounded_xpath_syntax() -> Result<()> {
        assert!(validate_xpath("//item[@value = '1']")?);
        assert!(!validate_xpath("[")?);
        assert!(matches!(
            validate_xpath(&format!("{}1", "-".repeat(65))),
            Err(PdfvError::Parse(ParseError::LimitExceeded {
                limit: "max_xmp_xpath_operators",
            }))
        ));
        Ok(())
    }
}
