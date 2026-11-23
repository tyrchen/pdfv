//! Scope the profile header checks to `ICCBased` colour spaces and output intents.

use std::{collections::BTreeSet, sync::Arc};

use super::{
    GenericModel, IccHeader, ModelGraph, ModelObjectRef, array_values, optional_string_model_value,
    optional_u64_model_value, parse_icc_header, push_linked, resolve_dictionary_value,
    semantic_names, stream_from_value, unknown_property,
};
use crate::{CosObject, Dictionary, ModelValue, ObjectKey, PropertyName, Result, ValidationError};

pub(super) const INPUT_FAMILY: &str = "iccInputProfile";
pub(super) const OUTPUT_FAMILY: &str = "iccOutputProfile";

pub(super) fn append<'a>(
    graph: &ModelGraph<'a>,
    output: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let mut scoped = BTreeSet::new();
    for object in graph.document.objects.values() {
        let Some(dictionary) = object.object.as_dictionary() else {
            continue;
        };
        if graph.materializes(OUTPUT_FAMILY)
            && semantic_names::name(graph.document, dictionary.get("Type"))
                .is_some_and(|name| name.matches("Catalog") || name.matches("Page"))
        {
            let intents = semantic_names::resolve(graph.document, dictionary.get("OutputIntents"));
            for value in array_values(intents) {
                if let Some(intent) = resolve_dictionary_value(graph.document, Some(value))
                    && let Some(CosObject::Reference(key)) = intent.get("DestOutputProfile")
                {
                    scoped.insert((OUTPUT_FAMILY, *key));
                }
            }
        }
        if graph.materializes(INPUT_FAMILY) {
            collect_inputs(graph, dictionary, &mut scoped)?;
        }
    }
    for (ordinal, (family, key)) in scoped.into_iter().enumerate() {
        let Some(object) = graph.document.objects.get(&key) else {
            continue;
        };
        let CosObject::Stream(stream) = &object.object else {
            continue;
        };
        let mut limits = graph.limits.clone();
        limits.max_stream_decode_bytes = limits
            .max_stream_decode_bytes
            .min(limits.max_icc_profile_bytes);
        let decoded = stream.decoded_bytes(&limits)?;
        let mut model = GenericModel::new(
            graph.document,
            family,
            Some(key),
            Some(object.offset),
            &stream.dictionary,
            ordinal,
            format!("root/{family}[{ordinal}]"),
        );
        model.supertypes.clear();
        model.icc_header = parse_icc_header(&decoded).map(Arc::new);
        push_linked(output, ModelObjectRef::Generic(model), max)?;
    }
    Ok(())
}

fn collect_inputs<'a>(
    graph: &ModelGraph<'a>,
    dictionary: &'a Dictionary,
    scoped: &mut BTreeSet<(&'static str, ObjectKey)>,
) -> Result<()> {
    let document = graph.document;
    if let Some(resources) = resolve_dictionary_value(document, dictionary.get("Resources")) {
        if let Some(spaces) = resolve_dictionary_value(document, resources.get("ColorSpace")) {
            for (_, value) in spaces.iter() {
                collect_space(graph, Some(value), scoped)?;
            }
        }
        for key in ["Shading", "Pattern"] {
            if let Some(entries) = resolve_dictionary_value(document, resources.get(key)) {
                for (_, value) in entries.iter() {
                    if let Some(entry) = resolve_dictionary_value(document, Some(value)) {
                        if key == "Shading" {
                            collect_space(graph, entry.get("ColorSpace"), scoped)?;
                        } else {
                            collect_pattern_shading(graph, entry, scoped)?;
                        }
                    }
                }
            }
        }
    }
    collect_pattern_shading(graph, dictionary, scoped)?;
    collect_space(graph, dictionary.get("ColorSpace"), scoped)?;
    if let Some(group) = resolve_dictionary_value(document, dictionary.get("Group"))
        && semantic_names::name(document, group.get("S"))
            .is_some_and(|kind| kind.matches("Transparency"))
    {
        collect_space(graph, group.get("CS"), scoped)?;
    }
    Ok(())
}

fn collect_pattern_shading(
    graph: &ModelGraph<'_>,
    pattern: &Dictionary,
    scoped: &mut BTreeSet<(&'static str, ObjectKey)>,
) -> Result<()> {
    let document = graph.document;
    if semantic_names::resolve(document, pattern.get("PatternType")).and_then(super::cos_integer)
        == Some(2)
        && let Some(shading) = resolve_dictionary_value(document, pattern.get("Shading"))
    {
        collect_space(graph, shading.get("ColorSpace"), scoped)?;
    }
    Ok(())
}

fn collect_space<'a>(
    graph: &ModelGraph<'a>,
    mut candidate: Option<&'a CosObject>,
    scoped: &mut BTreeSet<(&'static str, ObjectKey)>,
) -> Result<()> {
    let mut visited = BTreeSet::new();
    for _ in 0..=graph.limits.max_object_depth {
        if let Some(CosObject::Reference(key)) = candidate
            && !visited.insert(*key)
        {
            return Ok(());
        }
        let Some(CosObject::Array(space)) = semantic_names::resolve(graph.document, candidate)
        else {
            return Ok(());
        };
        let Some(CosObject::Name(family)) = space.first() else {
            return Ok(());
        };
        candidate = match family.as_bytes() {
            b"ICCBased" => {
                let Some(CosObject::Reference(key)) = space.get(1) else {
                    return Ok(());
                };
                if !scoped.insert((INPUT_FAMILY, *key)) {
                    return Ok(());
                }
                stream_from_value(graph.document, space.get(1))
                    .and_then(|stream| stream.dictionary.get("Alternate"))
            }
            b"Indexed" | b"Pattern" => space.get(1),
            b"Separation" | b"DeviceN" => space.get(2),
            _ => return Ok(()),
        };
    }
    Err(ValidationError::LimitExceeded {
        limit: "max_object_depth",
    }
    .into())
}

pub(super) fn property(
    document: &crate::ParsedDocument,
    dictionary: &Dictionary,
    header: Option<&IccHeader>,
    name: &PropertyName,
) -> Result<ModelValue> {
    let text =
        |field: fn(&IccHeader) -> &String| optional_string_model_value(header.map(field).cloned());
    match name.as_str() {
        "deviceClass" => Ok(text(|header| &header.device_class)),
        "colorSpace" => Ok(text(|header| &header.color_space)),
        "pcs" => Ok(text(|header| &header.pcs)),
        "N" => super::optional_i64_model_value(
            semantic_names::resolve(document, dictionary.get("N")).and_then(super::cos_integer),
        ),
        "version" => Ok(header.map_or(ModelValue::Null, |header| {
            let (major, minor, bugfix) = header.version_number;
            ModelValue::Number(
                f64::from(major) + f64::from(minor) / 10.0 + f64::from(bugfix) / 100.0,
            )
        })),
        "profileSize" => optional_u64_model_value(header.map(|header| header.profile_size)),
        "renderingIntent" => optional_u64_model_value(header.map(|header| header.rendering_intent)),
        "tagCount" => optional_u64_model_value(header.map(|header| header.tag_count)),
        _ => unknown_property(name),
    }
}
