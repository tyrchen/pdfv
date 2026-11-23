//! Rule objects for declared RDF extension-schema resources.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    XMP_NS_PDFA_EXTENSION, XMP_NS_PDFA_FIELD, XMP_NS_PDFA_PROPERTY, XMP_NS_PDFA_SCHEMA,
    XMP_NS_PDFA_TYPE, XmpRdfPropertyView, unknown_property, xmp_rdf_properties,
};
use crate::{
    BoundedText, Identifier, ModelValue, ObjectKey, ParseFact, ParsedDocument, PropertyName,
    Result, XmpExtensionRole, XmpFact,
};

pub(super) const FAMILIES: &[&str] = &[
    "extensionSchemasContainer",
    "extensionSchemaDefinition",
    "extensionSchemaProperty",
    "extensionSchemaValueType",
    "extensionSchemaField",
];

#[derive(Clone, Debug)]
pub(super) struct ExtensionResource<'a> {
    pub(super) id: u64,
    role: XmpExtensionRole,
    prefix: &'a Identifier,
    fields: Vec<XmpRdfPropertyView<'a>>,
    defined_types: Arc<BTreeSet<String>>,
}

pub(super) fn resources(document: &ParsedDocument, key: ObjectKey) -> Vec<ExtensionResource<'_>> {
    let properties = xmp_rdf_properties(document, Some(key)).collect::<Vec<_>>();
    let scopes = document
        .parse_facts
        .iter()
        .filter_map(|fact| {
            let ParseFact::Xmp {
                object,
                fact:
                    XmpFact::ExtensionScope {
                        id,
                        role,
                        schema,
                        prefix,
                        property_indices,
                    },
            } = fact
            else {
                return None;
            };
            (*object == key).then(|| {
                let fields = property_indices
                    .iter()
                    .filter_map(|index| properties.get(*index).cloned())
                    .collect::<Vec<_>>();
                (*id, *role, *schema, prefix, fields)
            })
        })
        .collect::<Vec<_>>();
    let mut type_names = BTreeMap::<u64, BTreeSet<String>>::new();
    for (_, role, schema, _, fields) in &scopes {
        if let (XmpExtensionRole::ValueType, Some(schema)) = (role, schema)
            && let Some(value) = select_field(fields, XMP_NS_PDFA_TYPE, "type")
                .0
                .and_then(|field| field.value)
        {
            type_names
                .entry(*schema)
                .or_default()
                .insert(value.as_str().to_ascii_lowercase());
        }
    }
    let type_names = type_names
        .into_iter()
        .map(|(id, types)| (id, Arc::new(types)))
        .collect::<BTreeMap<_, _>>();
    let empty = Arc::new(BTreeSet::new());
    scopes
        .into_iter()
        .map(|(id, role, schema, prefix, fields)| ExtensionResource {
            id,
            role,
            prefix,
            fields,
            defined_types: Arc::clone(schema.and_then(|id| type_names.get(&id)).unwrap_or(&empty)),
        })
        .collect()
}

pub(super) fn declared_value_types_valid(
    document: &ParsedDocument,
    key: ObjectKey,
) -> Option<bool> {
    let resources = resources(document, key);
    (!resources.is_empty()).then(|| {
        resources.iter().all(|resource| {
            let namespace = match resource.role {
                XmpExtensionRole::Property => XMP_NS_PDFA_PROPERTY,
                XmpExtensionRole::Field => XMP_NS_PDFA_FIELD,
                _ => return true,
            };
            let (field, duplicate) = select_field(&resource.fields, namespace, "valueType");
            !duplicate
                && field
                    .and_then(|field| field.value)
                    .is_some_and(|value| resource.value_type_defined(value.as_str()))
        })
    })
}

impl ExtensionResource<'_> {
    fn value_type_defined(&self, value: &str) -> bool {
        let name = base_value_type(value.trim());
        super::is_predefined_xmp_value_type(name)
            || self.defined_types.contains(&name.to_ascii_lowercase())
    }

    pub(super) fn family(&self) -> &'static str {
        match self.role {
            XmpExtensionRole::Container => "extensionSchemasContainer",
            XmpExtensionRole::Definition => "extensionSchemaDefinition",
            XmpExtensionRole::Property => "extensionSchemaProperty",
            XmpExtensionRole::ValueType => "extensionSchemaValueType",
            XmpExtensionRole::Field => "extensionSchemaField",
        }
    }

    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        let namespace = match self.role {
            XmpExtensionRole::Container => XMP_NS_PDFA_EXTENSION,
            XmpExtensionRole::Definition => XMP_NS_PDFA_SCHEMA,
            XmpExtensionRole::Property => XMP_NS_PDFA_PROPERTY,
            XmpExtensionRole::ValueType => XMP_NS_PDFA_TYPE,
            XmpExtensionRole::Field => XMP_NS_PDFA_FIELD,
        };
        let field = |local| select_field(&self.fields, namespace, local).0;
        let text = |local| field(local).and_then(|value| value.value);
        let has_text = |local| text(local).is_some_and(|value| !value.as_str().trim().is_empty());
        let prefix = |local| {
            field(local).map_or(ModelValue::Null, |value| {
                ModelValue::String(BoundedText::unchecked(value.prefix.as_str()))
            })
        };
        let array = |local, kind, optional| {
            let (value, duplicate) = select_field(&self.fields, namespace, local);
            !duplicate
                && value.map_or(optional, |value| {
                    value
                        .array_kind
                        .is_some_and(|array_kind| array_kind.as_str() == kind)
                })
        };
        let value = match name.as_str() {
            "prefix" => ModelValue::String(BoundedText::unchecked(self.prefix.as_str())),
            "isValidBag" => ModelValue::Bool(array("schemas", "Bag", false)),
            "isSchemaValidText" => ModelValue::Bool(has_text("schema")),
            "schemaPrefix" => prefix("schema"),
            "isNamespaceURIValidURI" => ModelValue::Bool(
                field("namespaceURI")
                    .is_some_and(|field| field.array_kind.is_none() && field.value.is_some()),
            ),
            "namespaceURIPrefix" => prefix("namespaceURI"),
            "isPrefixValidText" => ModelValue::Bool(has_text("prefix")),
            "prefixPrefix" => prefix("prefix"),
            "isPropertyValidSeq" => ModelValue::Bool(array("property", "Seq", true)),
            "propertyPrefix" => prefix("property"),
            "isValueTypeValidSeq" => ModelValue::Bool(array("valueType", "Seq", true)),
            "isValueTypeValidText" => ModelValue::Bool(has_text("valueType")),
            "valueTypePrefix" => prefix("valueType"),
            "isFieldValidSeq" => ModelValue::Bool(array("field", "Seq", true)),
            "fieldPrefix" => prefix("field"),
            "isTypeValidText" => ModelValue::Bool(has_text("type")),
            "typePrefix" => prefix("type"),
            "isNameValidText" => ModelValue::Bool(has_text("name")),
            "namePrefix" => prefix("name"),
            "isCategoryValidText" => ModelValue::Bool(has_text("category")),
            "categoryPrefix" => prefix("category"),
            "category" => text("category")
                .cloned()
                .map_or(ModelValue::Null, ModelValue::String),
            "isDescriptionValidText" => ModelValue::Bool(has_text("description")),
            "descriptionPrefix" => prefix("description"),
            "isValueTypeDefined" => ModelValue::Bool(
                text("valueType").is_some_and(|value| self.value_type_defined(value.as_str())),
            ),
            _ => return unknown_property(name),
        };
        Ok(value)
    }
}

fn select_field<'a, 'data>(
    fields: &'a [XmpRdfPropertyView<'data>],
    namespace: &str,
    local: &str,
) -> (Option<&'a XmpRdfPropertyView<'data>>, bool) {
    let mut matching = fields
        .iter()
        .filter(|field| field.namespace_uri.as_str() == namespace && field.name.as_str() == local);
    let first = matching.next();
    if matching.next().is_some() {
        (None, true)
    } else {
        (first, false)
    }
}

fn base_value_type(mut value: &str) -> &str {
    while let Some((head, tail)) = value.split_once(' ') {
        if ["Seq", "Bag", "Alt"]
            .iter()
            .any(|kind| head.eq_ignore_ascii_case(kind))
        {
            value = tail.trim_start();
        } else {
            break;
        }
    }
    value
}
