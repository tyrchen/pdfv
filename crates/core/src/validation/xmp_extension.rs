//! Rule objects for declared RDF extension-schema resources.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    XMP_NS_PDFA_EXTENSION, XMP_NS_PDFA_FIELD, XMP_NS_PDFA_PROPERTY, XMP_NS_PDFA_SCHEMA,
    XMP_NS_PDFA_TYPE, XmpRdfPropertyView, unknown_property, xmp_packet_properties,
};
use crate::{
    BoundedText, Identifier, ModelValue, ObjectKey, ParseFact, ParsedDocument, PropertyName,
    Result, XmpExtensionRole, XmpFact,
};

pub(crate) const OBJECT_FAMILY: &str = "extensionSchemaObject";
pub(super) const OBJECT_PROPERTIES: &[&str] = &["containsUndefinedFields", "undefinedFields"];
pub(super) const FAMILIES: &[&str] = &[
    OBJECT_FAMILY,
    "extensionSchemasContainer",
    "extensionSchemaDefinition",
    "extensionSchemaProperty",
    "extensionSchemaValueType",
    "extensionSchemaField",
];

pub(super) type CatalogTypes = BTreeMap<String, Arc<BTreeSet<String>>>;

#[derive(Clone, Debug)]
pub(super) struct ExtensionResource<'a> {
    pub(super) id: u64,
    role: XmpExtensionRole,
    prefix: &'a Identifier,
    fields: Vec<XmpRdfPropertyView<'a>>,
    defined_types: Arc<BTreeSet<String>>,
    inherited_types: Option<Arc<BTreeSet<String>>>,
    part: u32,
}

pub(super) fn resources<'a>(
    facts: &'a [ParseFact],
    key: ObjectKey,
    part: u32,
    catalog_types: Option<&CatalogTypes>,
) -> Vec<ExtensionResource<'a>> {
    let properties = xmp_packet_properties(facts, key).collect::<Vec<_>>();
    let scopes = facts
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
    let inherited_types: BTreeMap<_, _> = scopes
        .iter()
        .filter_map(|(id, role, _, _, fields)| {
            if *role != XmpExtensionRole::Definition {
                return None;
            }
            let namespace = declaration_namespace(fields)?;
            Some((*id, catalog_types?.get(namespace)?))
        })
        .collect();
    scopes
        .into_iter()
        .map(|(id, role, schema, prefix, fields)| ExtensionResource {
            id,
            role,
            prefix,
            fields,
            defined_types: Arc::clone(schema.and_then(|id| type_names.get(&id)).unwrap_or(&empty)),
            inherited_types: schema
                .and_then(|id| inherited_types.get(&id))
                .map(|types| Arc::clone(*types)),
            part,
        })
        .collect()
}

pub(super) fn catalog_types(facts: &[ParseFact], key: ObjectKey) -> CatalogTypes {
    resources(facts, key, 2, None)
        .into_iter()
        .filter_map(|resource| {
            if resource.role != XmpExtensionRole::Definition
                || select_field(&resource.fields, XMP_NS_PDFA_SCHEMA, "property")
                    .0
                    .is_none_or(|field| field.array_kind.is_none())
            {
                return None;
            }
            Some((
                declaration_namespace(&resource.fields)?.to_owned(),
                resource.defined_types,
            ))
        })
        .collect()
}

fn declaration_namespace<'a>(fields: &[XmpRdfPropertyView<'a>]) -> Option<&'a str> {
    select_field(fields, XMP_NS_PDFA_SCHEMA, "namespaceURI")
        .0?
        .value
        .map(BoundedText::as_str)
}

pub(super) fn declared_value_types_valid(
    document: &ParsedDocument,
    key: ObjectKey,
) -> Option<bool> {
    let resources = resources(&document.parse_facts, key, 2, None);
    (!resources.is_empty()).then(|| {
        resources.iter().all(|resource| {
            let namespace = match resource.role {
                XmpExtensionRole::Property => XMP_NS_PDFA_PROPERTY,
                XmpExtensionRole::Field => XMP_NS_PDFA_FIELD,
                _ => return true,
            };
            let (field, duplicate) = select_field(&resource.fields, namespace, "valueType");
            !duplicate
                && field.and_then(|field| field.value).is_some_and(|value| {
                    resource.value_type_defined(value.as_str()).unwrap_or(false)
                })
        })
    })
}

impl ExtensionResource<'_> {
    fn value_type_defined(&self, value: &str) -> Result<bool> {
        crate::xmp::property::extension_value_type_defined(
            self.part,
            value,
            &self.defined_types,
            self.inherited_types.as_deref(),
        )
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

    pub(super) fn is_schema_object(&self) -> bool {
        self.role != XmpExtensionRole::Container
    }

    fn allowed_fields(&self) -> (&'static str, &'static [&'static str]) {
        match self.role {
            XmpExtensionRole::Container => (XMP_NS_PDFA_EXTENSION, &["schemas"]),
            XmpExtensionRole::Definition => (
                XMP_NS_PDFA_SCHEMA,
                &["schema", "namespaceURI", "prefix", "property", "valueType"],
            ),
            XmpExtensionRole::Property => (
                XMP_NS_PDFA_PROPERTY,
                &["name", "valueType", "category", "description"],
            ),
            XmpExtensionRole::ValueType => (
                XMP_NS_PDFA_TYPE,
                &["type", "namespaceURI", "prefix", "description", "field"],
            ),
            XmpExtensionRole::Field => (XMP_NS_PDFA_FIELD, &["name", "valueType", "description"]),
        }
    }

    fn undefined_fields(&self) -> impl Iterator<Item = &XmpRdfPropertyView<'_>> {
        let (namespace, allowed) = self.allowed_fields();
        self.fields.iter().filter(move |field| {
            field.namespace_uri.as_str() != namespace || !allowed.contains(&field.name.as_str())
        })
    }

    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        let (namespace, _) = self.allowed_fields();
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
            "containsUndefinedFields" => ModelValue::Bool(self.undefined_fields().next().is_some()),
            "undefinedFields" => ModelValue::String(field_names(self.undefined_fields())),
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
            "isValueTypeDefined" => ModelValue::Bool(match text("valueType") {
                Some(value) => self.value_type_defined(value.as_str())?,
                None => false,
            }),
            _ => return unknown_property(name),
        };
        Ok(value)
    }
}

fn field_names<'a>(fields: impl Iterator<Item = &'a XmpRdfPropertyView<'a>>) -> BoundedText {
    let mut names = String::new();
    for field in fields {
        let name = field.name.as_str();
        let namespace = field.namespace_uri.as_str();
        let required = name.len().saturating_add(namespace.len()).saturating_add(3);
        if names.len().saturating_add(required) > 509 {
            names.push('…');
            break;
        }
        if !names.is_empty() {
            names.push(',');
        }
        names.push_str(name);
        names.push('(');
        names.push_str(namespace);
        names.push(')');
    }
    BoundedText::unchecked(names)
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
