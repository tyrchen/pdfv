//! Language-string checks scoped to a document-model use site.

use std::{collections::HashSet, sync::Arc};

use sha2::{Digest, Sha256};

use super::{
    ContentStreamModel, GenericModel, ModelGraph, ModelObjectRef, ResourceContent, catalog_value,
    glyphs::shown_fonts, push_linked, resolve_dictionary_value, resource_content_stream,
    scoped_pua, semantic_names, tagged_content,
};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ParsedDocument,
    PropertyName, Result, ValidationError,
    content::{ContentStreamSummary, InlineLanguage},
    pdf_text,
};

pub(crate) const FAMILY: &str = "cosLang";
pub(super) const CONTENT_FAMILY: &str = "languageContentStream";
pub(super) const PROPERTIES: &[&str] = &[
    "unicodeValue",
    "isLanguageTag",
    "isLanguageTagOrEmpty",
    "isAlphabeticLanguageTagOrEmpty",
    "containsPUA",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StreamView {
    Ordinary,
    Semantic,
    CharacterProcedure,
}

#[derive(Clone, Debug)]
pub(super) struct CharacterResources {
    pub(super) font: String,
    pub(super) dictionary: Option<Arc<Dictionary>>,
}

pub(super) fn enabled(graph: &ModelGraph<'_>) -> bool {
    graph.active_flavour.is_some()
        && (graph.materializes(FAMILY) || graph.materializes(scoped_pua::TEXT_STRING))
}

pub(super) fn semantic_content_enabled(graph: &ModelGraph<'_>) -> bool {
    (enabled(graph) || scoped_pua::replacement_enabled(graph))
        && graph.text_content_mode.has_semantic()
}

pub(super) fn read(graph: &ModelGraph<'_>, value: Option<&CosObject>) -> Result<Option<String>> {
    match semantic_names::resolve(graph.document, value) {
        Some(CosObject::String(text)) => decode(graph, text.as_bytes()).map(Some),
        _ => Ok(None),
    }
}

fn decode(graph: &ModelGraph<'_>, bytes: &[u8]) -> Result<String> {
    graph.charge_text_bytes(bytes.len())?;
    Ok(pdf_text::decode(bytes))
}

pub(super) fn append<'a>(
    graph: &ModelGraph<'a>,
    value: &str,
    owner: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    push_linked(
        objects,
        ModelObjectRef::Generic(model(graph, value, owner)),
        max_objects,
    )
}

pub(super) fn append_operator<'a>(
    graph: &ModelGraph<'a>,
    semantic: bool,
    value: &str,
    owner: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let mut model = model(graph, value, owner);
    model.language_operator = owner
        .object
        .zip(owner.offset)
        .map(|(key, offset)| (key, offset, semantic));
    push_linked(objects, ModelObjectRef::Generic(model), max_objects)
}

pub(super) fn append_content<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'_>,
    summary: &ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if summary.truncated || summary.scoped_operands_truncated {
        return Err(ValidationError::LimitExceeded {
            limit: "content_language_evidence",
        }
        .into());
    }
    let document = graph.document;
    let resources = content.effective_resources(document)?;
    let semantic = semantic_content_enabled(graph)
        && (content.language_view == StreamView::Semantic || !content.is_form);
    for span in &summary.marked_content {
        let value = match &span.inline_language {
            Some(InlineLanguage::Text(text)) => Some(decode(graph, text.as_bytes())?),
            Some(InlineLanguage::Reference(key)) => read(graph, Some(&CosObject::Reference(*key)))?,
            None => read(
                graph,
                tagged_content::property_list(document, &resources, span)
                    .and_then(|dictionary| dictionary.get("Lang")),
            )?,
        };
        if let Some(value) = value {
            append_operator(
                graph,
                semantic,
                &value,
                &span.location,
                objects,
                max_objects,
            )?;
        }
    }
    Ok(())
}

fn model<'a>(graph: &ModelGraph<'a>, value: &str, owner: &ObjectLocation) -> GenericModel<'a> {
    let context = owner.path.as_ref().map_or("root", BoundedText::as_str);
    let mut model = GenericModel::new_value(
        graph.document,
        FAMILY,
        ModelValue::String(BoundedText::unchecked(value)),
        0,
        format!("{context}/Lang[0]"),
    );
    model.key = owner.object;
    model.offset = owner.offset;
    model.supertypes = vec![crate::ObjectTypeName::unchecked(scoped_pua::TEXT_STRING)];
    model
}

pub(super) fn append_type3_streams<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'a>,
    summary: &ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !summary.has_text() {
        return Ok(());
    }
    let document = graph.document;
    let resources = content.effective_resources(document)?;
    let fonts = shown_fonts(content, document, &resources, summary, graph.limits)?;
    for selection in fonts {
        let Some(font) = selection.dictionary(document) else {
            continue;
        };
        if !semantic_names::name(document, font.get("Subtype"))
            .is_some_and(|name| name.matches("Type3"))
        {
            continue;
        }
        let Some(procedures) = resolve_dictionary_value(document, font.get("CharProcs")) else {
            continue;
        };
        if u64::try_from(procedures.len()).unwrap_or(u64::MAX)
            > graph.limits.max_type3_charproc_streams
        {
            return Err(ValidationError::LimitExceeded {
                limit: "max_type3_charproc_streams",
            }
            .into());
        }
        let identity = selection.identity();
        let resources = graph
            .character_resources
            .try_borrow_mut()
            .map_err(|_| ValidationError::SubsystemUnavailable {
                subsystem: "language_resources",
            })?
            .entry(identity.clone())
            .or_insert_with(|| {
                resolve_dictionary_value(document, font.get("Resources"))
                    .map(|resources| Arc::new(resources.clone()))
            })
            .clone();
        let resources = CharacterResources {
            font: identity,
            dictionary: resources,
        };
        for (_, value) in procedures.iter() {
            let CosObject::Reference(key) = value else {
                continue;
            };
            let location_path = format!(
                "root/page[{}]/type3CharProc[{}.{}]",
                content.page_ordinal, key.number, key.generation
            );
            if let Some(model) = resource_content_stream(
                graph,
                *key,
                content.page_key,
                content.page_ordinal,
                location_path,
                ResourceContent::CharProc(resources.clone()),
            )? {
                push_linked(objects, ModelObjectRef::ContentStream(model), max_objects)?;
            }
        }
    }
    Ok(())
}

pub(super) fn append_form_fields<'a>(
    graph: &ModelGraph<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let document = graph.document;
    let Some(acro_form) = resolve_dictionary_value(document, catalog_value(document, "AcroForm"))
    else {
        return Ok(());
    };
    let mut fields = Vec::new();
    push_fields(
        document,
        acro_form.get("Fields"),
        &mut fields,
        max_objects,
        false,
    )?;
    let mut visited = HashSet::new();
    let mut scanned = 0_usize;
    while let Some((value, require_name)) = fields.pop() {
        let key = match value {
            CosObject::Reference(key) if visited.insert(*key) => Some(*key),
            CosObject::Reference(_) => continue,
            _ => None,
        };
        if scanned >= max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let ordinal = scanned;
        scanned = scanned.saturating_add(1);
        let Some(dictionary) = resolve_dictionary_value(document, Some(value)) else {
            continue;
        };
        if require_name
            && dictionary
                .get("T")
                .is_none_or(|value| matches!(value, CosObject::Null))
        {
            continue;
        }
        let location = ObjectLocation {
            object: key,
            offset: key
                .and_then(|key| document.objects.get(&key))
                .map(|object| object.offset),
            path: Some(BoundedText::unchecked(format!(
                "root/document[0]/AcroForm[0]/formField[{ordinal}]"
            ))),
        };
        append_parent_language(graph, dictionary, &location, objects, max_objects)?;
        push_fields(
            document,
            dictionary.get("Kids"),
            &mut fields,
            max_objects,
            true,
        )?;
    }
    Ok(())
}

fn push_fields<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
    fields: &mut Vec<(&'a CosObject, bool)>,
    max_objects: usize,
    require_name: bool,
) -> Result<()> {
    if let Some(CosObject::Array(values)) = semantic_names::resolve(document, value) {
        if values.len() > max_objects.saturating_sub(fields.len()) {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        fields.extend(values.iter().rev().map(|value| (value, require_name)));
    }
    Ok(())
}

pub(super) fn operator_identity(source: (ObjectKey, u64, bool), value: &ModelValue) -> String {
    let (key, offset, semantic) = source;
    let text = match value {
        ModelValue::String(text) => text.as_str(),
        _ => "",
    };
    let language: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    format!(
        "{FAMILY}:operator:{}:{}:{offset}:{semantic}:{language:02x?}",
        key.number, key.generation
    )
}

pub(super) fn append_parent_language<'a>(
    graph: &ModelGraph<'a>,
    dictionary: &Dictionary,
    location: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if !enabled(graph) || dictionary.get("StructParent").is_none() {
        return Ok(());
    }
    let accessibility = graph.accessibility_graph()?;
    if let Some(parent) = accessibility.content_parent(graph.document, dictionary, None)?
        && let Some(value) = graph
            .document
            .objects
            .get(&parent)
            .and_then(|object| object.object.as_dictionary())
        && let Some(value) = read(graph, value.get("Lang"))?
    {
        append(graph, &value, location, objects, max_objects)?;
    }
    Ok(())
}

pub(super) fn property(value: &BoundedText, name: &PropertyName) -> Option<ModelValue> {
    let value = value.as_str();
    let result = match name.as_str() {
        "unicodeValue" => return Some(ModelValue::String(BoundedText::unchecked(value))),
        "isLanguageTag" => is_language_tag(value, true),
        "isLanguageTagOrEmpty" => value.is_empty() || is_language_tag(value, true),
        "isAlphabeticLanguageTagOrEmpty" => value.is_empty() || is_language_tag(value, false),
        "containsPUA" => value.chars().any(pdf_text::is_pua),
        _ => return None,
    };
    Some(ModelValue::Bool(result))
}

fn is_language_tag(value: &str, allow_digits: bool) -> bool {
    value.split('-').enumerate().all(|(index, part)| {
        (1..=8).contains(&part.len())
            && part.bytes().all(|byte| {
                byte.is_ascii_alphabetic() || (index > 0 && allow_digits && byte.is_ascii_digit())
            })
    })
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write, io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, ProfileReport, Result,
        ValidationFlavour, ValidationOptions, Validator,
    };

    const RULE: &str = "iso-14289-1-7-2-29";

    #[rstest]
    #[case("BT /T 10 Tf ET", "", 5)]
    #[case("BT /T 10 Tf (A) Tj ET", "", 6)]
    #[case("BT /T 10 Tf ET /Fm Do", "BT (A) Tj ET", 6)]
    fn test_should_apply_language_only_at_document_use_sites(
        #[case] type3_text: &str,
        #[case] form_text: &str,
        #[case] failed: usize,
    ) -> Result<()> {
        let mut pdf = String::from(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Lang 11 0 R /StructTreeRoot 5 0 R \
             /AcroForm << /Fields [7 0 R] >> /Other << /Lang (ignored_value) >> >>\nendobj\n2 0 \
             obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page \
             /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots [7 0 R]\n/Resources << \
             /Properties << /Named 8 0 R >> /XObject << /Fm 10 0 R /Unused 14 0 R >> /Font << /T \
             12 0 R >> >> >>\nendobj\n5 0 obj\n<< /Type /StructTreeRoot /K [6 0 R] /ParentTree << \
             /Nums [0 6 0 R] >> >>\nendobj\n6 0 obj\n<< /Type /StructElem /S /P /P 5 0 R /Lang \
             (bad_tag) >>\nendobj\n7 0 obj\n<< /Type /Annot /Subtype /Widget /StructParent 0 /FT \
             /Btn /Rect [0 0 20 20] /Lang (unused) /AP << /N 10 0 R >> >>\nendobj\n8 0 obj\n<< \
             /Lang 9 0 R >>\nendobj\n9 0 obj\n<EFBBBF656E2D5553>\nendobj\n11 0 \
             obj\n<FEFF006E006C>\nendobj\n12 0 obj\n<< /Type /Font /Subtype /Type3 /FirstChar 65 \
             /LastChar 65 /Widths [500]\n/FontBBox [0 0 500 500] /FontMatrix [0.001 0 0 0.001 0 \
             0] /Encoding << /Differences [65 /A] >> /CharProcs << /A 13 0 R >> >>\nendobj\n",
        );
        let page = format!("/Span /Named BDC EMC\n/Fm Do /Fm Do\n{type3_text}");
        push_stream(&mut pdf, 4, "", &page)?;
        let form = format!("1 /Span << /Lang (bad_tag) >> BDC EMC\n{form_text}");
        push_stream(
            &mut pdf,
            10,
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
            &form,
        )?;
        push_stream(
            &mut pdf,
            13,
            "",
            "500 0 d0 /Span << /Lang (bad_tag) >> BDC EMC",
        )?;
        push_stream(
            &mut pdf,
            14,
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
            "/Span << /Lang (unused_tag) >> BDC EMC",
        )?;
        pdf.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");

        let profile = validate(&pdf, "pdfua", "none")?;
        let passes = profile
            .passed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == RULE)
            .count();
        let failures = profile
            .failed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == RULE)
            .count();
        assert_eq!((passes, failures), (2, failed));
        assert!(
            !profile
                .unsupported_rules
                .iter()
                .any(|item| item.rule_id.0.as_str() == RULE)
        );
        Ok(())
    }

    #[rstest]
    #[case("pdfua", "none", RULE, "<FEFF>", false)]
    #[case("pdfua", "none", RULE, "(en-419)", true)]
    #[case("pdfa", "a", "iso-19005-1-6-8-4-1", "<EFBBBF>", true)]
    #[case("pdfa", "a", "iso-19005-1-6-8-4-1", "(en-419)", false)]
    fn test_should_apply_profile_specific_language_syntax(
        #[case] family: &str,
        #[case] level: &str,
        #[case] rule: &str,
        #[case] lang: &str,
        #[case] passes: bool,
    ) -> Result<()> {
        let pdf = format!(
            "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Lang {lang} >>\nendobj\ntrailer\n<< /Root 1 0 \
             R >>\n%%EOF\n"
        );
        let profile = validate(&pdf, family, level)?;
        let assertions = if passes {
            profile.passed_assertions
        } else {
            profile.failed_assertions
        };
        assert!(
            assertions
                .iter()
                .any(|item| item.rule_id.0.as_str() == rule)
        );
        Ok(())
    }

    fn validate(pdf: &str, family: &str, level: &str) -> Result<ProfileReport> {
        let flavour = ValidationFlavour::new(family, NonZeroU32::MIN, level)?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        Validator::new(options)?
            .validate_reader(Cursor::new(pdf.as_bytes()), InputName::memory())?
            .profile_reports
            .into_iter()
            .next()
            .ok_or_else(|| ProfileError::UnsupportedSelection.into())
    }

    fn push_stream(pdf: &mut String, number: u32, entries: &str, bytes: &str) -> Result<()> {
        write!(
            pdf,
            "{number} 0 obj\n<< {entries} /Length {} >>\nstream\n{bytes}\nendstream\nendobj\n",
            bytes.len()
        )
        .map_err(|_| ProfileError::UnsupportedSelection.into())
    }
}
