//! Metadata streams referenced from physical PDF dictionaries.

use std::collections::{BTreeMap, btree_map::Entry};

use super::{
    LinkedXmpPacket, MetadataModel, ModelGraph, ModelObjectRef, catalog_metadata_key,
    cos::{CosPrimitive, WalkScope, walk_primitives},
    push_linked,
    semantic_names::resolve_reference,
    xmp_extension,
};
use crate::{
    CosObject, ObjectKey, ObjectTypeName, ParseError, ParseFact, ParsedDocument, ResourceLimits,
    Result, StreamObject, ValidationError, ValidationFlavour, ValidationProfile, ValidationWarning,
    XmpFact,
    xmp::{CatalogSchemas, PACKET_FAMILY, parse_object_xmp, property},
};

pub(super) fn applies(flavour: &ValidationFlavour) -> bool {
    flavour.family.as_str() == "pdfa" && matches!(flavour.part.get(), 2..=4)
}

pub(super) fn link_models<'a>(
    graph: &ModelGraph<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !graph.active_flavour.as_ref().is_some_and(applies) {
        return Ok(());
    }
    let facts = &graph.document.parse_facts;
    for (index, fact) in facts.iter().enumerate() {
        let ParseFact::Xmp {
            object,
            fact: XmpFact::LinkedMetadata { owner, fact_count },
        } = fact
        else {
            continue;
        };
        let unavailable = || ValidationError::SubsystemUnavailable {
            subsystem: "object_xmp_packet",
        };
        let start = index.checked_add(1).ok_or_else(unavailable)?;
        let end = start
            .checked_add(*fact_count)
            .filter(|end| *end <= facts.len())
            .ok_or_else(unavailable)?;
        let mut model =
            MetadataModel::new(graph.document, Some(*object), graph.active_flavour.as_ref())
                .ok_or_else(unavailable)?;
        model.linked_packet = Some(LinkedXmpPacket {
            owner: *owner,
            start,
            end,
        });
        model.object_type = ObjectTypeName::unchecked(PACKET_FAMILY);
        model.supertypes.clear();
        push_linked(objects, ModelObjectRef::Metadata(model), max_objects)?;
    }
    Ok(())
}

pub(super) fn append(
    document: &mut ParsedDocument,
    limits: &ResourceLimits,
    profiles: &[ValidationProfile],
    catalog: &CatalogSchemas,
    catalog_bytes: u64,
) -> Result<()> {
    if !profiles.iter().any(|profile| {
        applies(&profile.flavour)
            && profile.rules.iter().any(|rule| {
                let family = rule.object_type.as_str();
                matches!(family, PACKET_FAMILY | property::FAMILY)
                    || xmp_extension::FAMILIES.contains(&family)
            })
    }) {
        return Ok(());
    }
    let mut facts = Vec::new();
    let mut warnings = Vec::new();
    let mut budget = limits.clone();
    let mut bytes_left = limits.max_file_bytes.saturating_sub(catalog_bytes);
    let mut facts_left = limits
        .max_parse_facts
        .saturating_sub(document.parse_facts.len());
    let declarations = catalog.declarations();
    for (object, source) in sources(document, limits)? {
        let available = facts_left.checked_sub(1).ok_or(ParseError::LimitExceeded {
            limit: "max_parse_facts",
        })?;
        budget.max_parse_facts = available;
        budget.max_stream_decode_bytes = limits.max_stream_decode_bytes.min(bytes_left);
        let evidence = parse_object_xmp(object, source.stream, &budget, &declarations)?;
        if evidence
            .warnings
            .iter()
            .any(|warning| matches!(warning, ValidationWarning::ParseFactCapReached { .. }))
        {
            return Err(ParseError::LimitExceeded {
                limit: "max_parse_facts",
            }
            .into());
        }
        bytes_left =
            bytes_left
                .checked_sub(evidence.decoded_bytes)
                .ok_or(ParseError::LimitExceeded {
                    limit: "object_xmp_bytes",
                })?;
        facts_left =
            available
                .checked_sub(evidence.parse_facts.len())
                .ok_or(ParseError::LimitExceeded {
                    limit: "max_parse_facts",
                })?;
        facts.push(ParseFact::Xmp {
            object,
            fact: XmpFact::LinkedMetadata {
                owner: source.owner,
                fact_count: evidence.parse_facts.len(),
            },
        });
        facts.extend(evidence.parse_facts);
        warnings.extend(evidence.warnings);
    }
    document.parse_facts.extend(facts);
    document.warnings.extend(warnings);
    Ok(())
}

#[derive(Debug)]
struct Source<'a> {
    stream: &'a StreamObject,
    owner: Option<ObjectKey>,
}

fn sources<'a>(
    document: &'a ParsedDocument,
    limits: &ResourceLimits,
) -> Result<BTreeMap<ObjectKey, Source<'a>>> {
    let mut sources = BTreeMap::new();
    let primary = catalog_metadata_key(document);
    let limit = usize::try_from(limits.max_objects).unwrap_or(usize::MAX);
    let resolve = |value| resolve_reference(document, value, limits.max_object_depth);
    walk_primitives(
        document,
        limit,
        WalkScope::Containers,
        |primitive, origin| {
            let (CosPrimitive::Dictionary(dictionary)
            | CosPrimitive::StreamOrTrailerDictionary(dictionary)) = primitive
            else {
                return Ok(());
            };
            let Some((Some(key), CosObject::Stream(stream))) = resolve(dictionary.get("Metadata"))?
            else {
                return Ok(());
            };
            if Some(key) == primary
                || matches!(resolve(dictionary.get("Type"))?, Some((_, CosObject::Name(name))) if name.matches("Catalog"))
            {
                return Ok(());
            }
            if let Entry::Vacant(entry) = sources.entry(key)
                && matches!(resolve(stream.dictionary.get("Type"))?, Some((_, CosObject::Name(name))) if name.matches("Metadata"))
                && matches!(resolve(stream.dictionary.get("Subtype"))?, Some((_, CosObject::Name(name))) if name.matches("XML"))
            {
                entry.insert(Source {
                    stream,
                    owner: origin.owner,
                });
            }
            Ok(())
        },
    )?;
    Ok(sources)
}
