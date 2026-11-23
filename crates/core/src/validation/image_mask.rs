//! Explicit image-mask evidence from invoked image `XObjects`.

use super::{
    GenericModel, ModelGraph, ModelObjectRef, object_key_text, push_linked, semantic_names,
};
use crate::{CosObject, Dictionary, ModelValue, ObjectKey, Result, ValidationError};

pub(crate) const FAMILY: &str = "imageMask";
pub(crate) const INVOKED_IMAGE: &str = "invokedImage";
pub(super) const SOFT_MASK: &str = "invokedSoftMask";
pub(super) const PROPERTIES: &[&str] = &["BitsPerComponent", "isMask"];

pub(super) fn enabled(graph: &ModelGraph<'_>) -> bool {
    graph.active_flavour.is_some()
        && (graph.materializes(FAMILY) || graph.materializes(INVOKED_IMAGE))
}

pub(super) fn append_invoked<'a>(
    graph: &ModelGraph<'a>,
    value: &CosObject,
    path: &str,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let Some((key, value)) = semantic_names::resolve_reference(
        graph.document,
        Some(value),
        graph.limits.max_object_depth,
    )?
    else {
        return Ok(());
    };
    let Some(dictionary) = value.as_dictionary() else {
        return Ok(());
    };
    if matches!(
        semantic_names::resolve_reference(graph.document, dictionary.get("Subtype"), graph.limits.max_object_depth)?,
        Some((_, CosObject::Name(name))) if name.matches("Image")
    ) {
        let image = model(graph, INVOKED_IMAGE, key, dictionary, path)?;
        push_linked(objects, image, max_objects)?;
        if key.is_none() {
            append_image_links(graph, INVOKED_IMAGE, dictionary, path, objects, max_objects)?;
        }
    }
    Ok(())
}

pub(super) fn linked_objects<'a>(
    model: &GenericModel<'a>,
    graph: &ModelGraph<'a>,
    max_objects: usize,
) -> Result<Vec<ModelObjectRef<'a>>> {
    let mut objects = Vec::new();
    let Some((key, dictionary)) = model.key.and_then(|key| {
        let dictionary = graph.document.objects.get(&key)?.object.as_dictionary()?;
        Some((key, dictionary))
    }) else {
        return Ok(objects);
    };
    let owner = format!(
        "root/{}[{}]",
        model.object_type.as_str(),
        object_key_text(key)
    );
    append_image_links(
        graph,
        model.object_type.as_str(),
        dictionary,
        &owner,
        &mut objects,
        max_objects,
    )?;
    Ok(objects)
}

fn append_image_links<'a>(
    graph: &ModelGraph<'a>,
    owner_family: &str,
    dictionary: &Dictionary,
    path: &str,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    for (entry, family) in [("Mask", FAMILY), ("SMask", SOFT_MASK)] {
        if (family != FAMILY || owner_family == INVOKED_IMAGE)
            && let Some(value) = dictionary.get(entry)
            && let Some(key) = stream_key(graph, value)?
        {
            append(graph, family, key, path, objects, max_objects)?;
        }
    }
    if let Some(value) = dictionary.get("Alternates")
        && let Some((_, CosObject::Array(alternates))) = semantic_names::resolve_reference(
            graph.document,
            Some(value),
            graph.limits.max_object_depth,
        )?
    {
        if alternates.len() > max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        for alternate in alternates {
            if let Some((_, CosObject::Dictionary(dictionary))) = semantic_names::resolve_reference(
                graph.document,
                Some(alternate),
                graph.limits.max_object_depth,
            )? && let Some(value) = dictionary.get("Image")
                && let Some(key) = stream_key(graph, value)?
            {
                append(graph, INVOKED_IMAGE, key, path, objects, max_objects)?;
            }
        }
    }
    Ok(())
}

fn stream_key(graph: &ModelGraph<'_>, value: &CosObject) -> Result<Option<ObjectKey>> {
    match semantic_names::resolve_reference(
        graph.document,
        Some(value),
        graph.limits.max_object_depth,
    )? {
        Some((Some(key), CosObject::Stream(_))) => Ok(Some(key)),
        _ => Ok(None),
    }
}

fn append<'a>(
    graph: &ModelGraph<'a>,
    family: &'static str,
    key: ObjectKey,
    path: &str,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let Some(dictionary) = graph
        .document
        .objects
        .get(&key)
        .and_then(|object| object.object.as_dictionary())
    else {
        return Ok(());
    };
    push_linked(
        objects,
        model(graph, family, Some(key), dictionary, path)?,
        max_objects,
    )
}

fn model<'a>(
    graph: &ModelGraph<'a>,
    family: &'static str,
    key: Option<ObjectKey>,
    dictionary: &Dictionary,
    path: &str,
) -> Result<ModelObjectRef<'a>> {
    let depth = match semantic_names::resolve_reference(
        graph.document,
        dictionary.get("BitsPerComponent"),
        graph.limits.max_object_depth,
    )? {
        None if dictionary.get("BitsPerComponent").is_none() => ModelValue::Null,
        Some((_, CosObject::Null)) => ModelValue::Null,
        Some((_, CosObject::Integer(bits))) => bits_value(*bits),
        _ => ModelValue::Bool(false),
    };
    let context = key.map_or_else(
        || format!("{path}/{family}[inline]"),
        |key| format!("{path}/{family}[{}]", object_key_text(key)),
    );
    let mut model = GenericModel::new_value(graph.document, family, depth, 0, context);
    model.key = key;
    model.offset = key.and_then(|key| graph.document.objects.get(&key).map(|object| object.offset));
    model.supertypes.clear();
    if family != INVOKED_IMAGE {
        model
            .supertypes
            .push(crate::ObjectTypeName::unchecked(INVOKED_IMAGE));
    }
    Ok(ModelObjectRef::Generic(model))
}

pub(super) fn bits_value(bits: i64) -> ModelValue {
    u8::try_from(bits).map_or(ModelValue::Bool(false), |bits| {
        ModelValue::Number(f64::from(bits))
    })
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, Result, ValidationFlavour,
        ValidationOptions, Validator,
    };

    #[rstest]
    #[case::normal("/Im Do", "", "8", (1, 0), (0, 0))]
    #[case::unused("", "/Mask 6 0 R", "8", (0, 0), (0, 0))]
    #[case::hard("/Im Do /Im Do", "/Mask 6 0 R", "8", (2, 0), (0, 1))]
    #[case::hard_one("/Im Do", "/Mask 6 0 R", "1", (2, 0), (1, 0))]
    #[case::hard_null("/Im Do", "/Mask 6 0 R", "null", (2, 0), (1, 0))]
    #[case::hard_alias("/Im Do", "/Mask 7 0 R", "8", (2, 0), (0, 1))]
    #[case::hard_wrong_type("/Im Do", "/Mask 6 0 R", "(x)", (2, 0), (0, 1))]
    #[case::soft("/Im Do", "/SMask 6 0 R", "3", (1, 1), (0, 0))]
    #[case::color_key("/Im Do", "/Mask [1 2 3 4 5 6]", "8", (1, 0), (0, 0))]
    #[case::inline_stencil("BI /W 1 /H 1 /IM true /BPC -1 ID x EI", "", "8", (0, 1), (0, 0))]
    fn test_should_check_component_depth_in_the_actual_image_role(
        #[case] drawing: &str,
        #[case] mask_entry: &str,
        #[case] mask_bpc: &str,
        #[case] image_counts: (usize, usize),
        #[case] mask_counts: (usize, usize),
    ) -> Result<()> {
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R \
             >>\nendobj\n4 0 obj\n<< /Length {} >>\nstream\n{drawing}\nendstream\nendobj\n5 0 \
             obj\n<< /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace \
             /DeviceRGB /Length 3 {mask_entry} >>\nstream\nabc\nendstream\nendobj\n6 0 obj\n<< \
             /Subtype /Image /Width 1 /Height 1 /BitsPerComponent {mask_bpc} /Length 1 \
             >>\nstream\nx\nendstream\nendobj\n7 0 obj\n6 0 R\nendobj\ntrailer\n<< /Root 1 0 R \
             >>\n%%EOF\n",
            drawing.len(),
        );
        let part = NonZeroU32::new(3).ok_or(ProfileError::UnsupportedSelection)?;
        let flavour = ValidationFlavour::new("pdfa", part, "b")?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        let report = Validator::new(options)?
            .validate_reader(Cursor::new(pdf.as_bytes()), InputName::memory())?;
        let profile = report
            .profile_reports
            .first()
            .ok_or(ProfileError::UnsupportedSelection)?;
        for (rule, counts) in [
            ("iso-19005-3-6-2-8-4", image_counts),
            ("iso-19005-3-6-2-8-5", mask_counts),
        ] {
            let matches_rule = |assertion: &&crate::Assertion| assertion.rule_id.0.as_str() == rule;
            assert_eq!(
                (
                    profile
                        .passed_assertions
                        .iter()
                        .filter(matches_rule)
                        .count(),
                    profile
                        .failed_assertions
                        .iter()
                        .filter(matches_rule)
                        .count()
                ),
                counts,
                "{rule}"
            );
            assert!(
                !profile
                    .unsupported_rules
                    .iter()
                    .any(|item| item.rule_id.0.as_str() == rule)
            );
        }
        Ok(())
    }
}
