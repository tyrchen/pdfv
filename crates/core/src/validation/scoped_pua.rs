//! Private-use code point checks on text bound to PDF document-model use sites.

use std::collections::HashSet;

use super::{
    ContentStreamModel, GenericModel, ModelGraph, ModelObjectRef, catalog_value, language,
    push_linked, resolve_dictionary_value, tagged_content,
};
use crate::{
    BoundedText, CosObject, ModelValue, ObjectLocation, Result, ValidationError, ValidationProfile,
    content::ContentStreamSummary,
    pdf_text::{self, PuaSource, ReplacementText},
};

pub(crate) const ACTUAL_TEXT: &str = "cosActualText";
pub(crate) const ALT: &str = "cosAlt";
pub(crate) const TEXT_STRING: &str = "cosTextString";
pub(super) const FAMILIES: &[&str] = &[ACTUAL_TEXT, ALT, TEXT_STRING];
pub(super) const PROPERTIES: &[&str] = &["containsPUA"];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ContentMode {
    #[default]
    Ordinary,
    Semantic,
    Mixed,
}

impl ContentMode {
    pub(super) fn for_profiles(profiles: &[ValidationProfile]) -> Self {
        let accessibility = |profile: &ValidationProfile| {
            matches!(profile.flavour.family.as_str(), "pdfua" | "wtpdf")
        };
        if !profiles.iter().any(accessibility) {
            Self::Ordinary
        } else if profiles.iter().all(accessibility) {
            Self::Semantic
        } else {
            Self::Mixed
        }
    }

    pub(super) fn has_semantic(self) -> bool {
        self != Self::Ordinary
    }
}

fn bounded_source(
    graph: &ModelGraph<'_>,
    field: &'static str,
    value: Option<&CosObject>,
) -> Result<Option<PuaSource>> {
    if !graph.materializes(field) {
        return Ok(None);
    }
    if let Some(CosObject::String(text)) = value {
        graph.charge_text_bytes(text.as_bytes().len())?;
    }
    Ok(value.and_then(PuaSource::from_object))
}

pub(super) fn replacement_enabled(graph: &ModelGraph<'_>) -> bool {
    graph.active_flavour.is_some() && (graph.materializes(ACTUAL_TEXT) || graph.materializes(ALT))
}

pub(super) fn ordinary_content_enabled(graph: &ModelGraph<'_>) -> bool {
    replacement_enabled(graph) && graph.text_content_mode != ContentMode::Semantic
}

fn read(graph: &ModelGraph<'_>, source: PuaSource) -> Result<Option<bool>> {
    match source {
        PuaSource::Direct(contains_pua) => Ok(Some(contains_pua)),
        PuaSource::Reference(key) => {
            match graph
                .document
                .objects
                .get(&key)
                .map(|object| &object.object)
            {
                Some(CosObject::String(text)) => {
                    graph.charge_text_bytes(text.as_bytes().len())?;
                    Ok(Some(pdf_text::contains_pua(text.as_bytes())))
                }
                _ => Ok(None),
            }
        }
    }
}

fn append<'a>(
    graph: &ModelGraph<'a>,
    field: (&'static str, &str),
    source: Option<PuaSource>,
    location: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let (family, name) = field;
    if graph.materializes(family)
        && let Some(source) = source
        && let Some(contains_pua) = read(graph, source)?
    {
        let context = location.path.as_ref().map_or("root", BoundedText::as_str);
        let mut model = GenericModel::new_value(
            graph.document,
            family,
            ModelValue::Bool(contains_pua),
            0,
            format!("{context}/{name}[0]"),
        );
        model.key = location.object;
        model.offset = location.offset;
        model.supertypes.clear();
        push_linked(objects, ModelObjectRef::Generic(model), max_objects)?;
    }
    Ok(())
}

pub(super) fn append_replacements<'a>(
    graph: &ModelGraph<'a>,
    text: ReplacementText,
    location: &ObjectLocation,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    for (field, value) in [
        ((ACTUAL_TEXT, "ActualText"), text.actual_text),
        ((ALT, "Alt"), text.alt),
    ] {
        append(graph, field, value, location, objects, max_objects)?;
    }
    Ok(())
}

pub(super) fn append_content<'a>(
    graph: &ModelGraph<'a>,
    content: &ContentStreamModel<'_>,
    summary: &ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let ordinary = ordinary_content_enabled(graph);
    let semantic = graph.text_content_mode.has_semantic()
        && (!content.is_form || content.language_view == language::StreamView::Semantic);
    if !replacement_enabled(graph) || content.page_key.is_none() || !(ordinary || semantic) {
        return Ok(());
    }
    if summary.truncated
        || summary.scoped_operands_truncated
        || (ordinary && summary.marked_points_truncated)
    {
        return Err(ValidationError::LimitExceeded {
            limit: "content_replacement_text_evidence",
        }
        .into());
    }
    let resources = content.effective_resources(graph.document)?;
    for used in &summary.marked_text_uses {
        if used.point && !ordinary {
            continue;
        }
        let properties = tagged_content::resolve_property_list(
            graph.document,
            &resources,
            used.properties,
            used.properties_name.as_ref(),
        );
        let text = if let Some(properties) = properties {
            ReplacementText {
                actual_text: bounded_source(graph, ACTUAL_TEXT, properties.get("ActualText"))?,
                alt: bounded_source(graph, ALT, properties.get("Alt"))?,
            }
        } else {
            used.inline
        };
        // The same physical operator can be reached via several invocations. Preserve
        // distinct evidence when its effective property resources differ.
        let mut location = used.location.clone();
        if let Some((key, offset)) = location.object.zip(location.offset) {
            location.path = Some(BoundedText::unchecked(format!(
                "root/contentStream[{}.{}]/operator[{offset}]/{}",
                key.number,
                key.generation,
                if semantic { "semantic" } else { "ordinary" }
            )));
        }
        append_replacements(graph, text, &location, objects, max_objects)?;
    }
    Ok(())
}

pub(super) fn append_outline_titles<'a>(
    graph: &ModelGraph<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    let document = graph.document;
    let Some(outlines) = resolve_dictionary_value(document, catalog_value(document, "Outlines"))
    else {
        return Ok(());
    };
    let mut pending: Vec<_> = outlines.get("First").into_iter().collect();
    let mut seen = HashSet::new();
    let mut ordinal = 0_usize;
    let budget = max_objects.saturating_sub(objects.len());
    while let Some(value) = pending.pop() {
        let key = match value {
            CosObject::Reference(key) if seen.insert(*key) => Some(*key),
            CosObject::Reference(_) => continue,
            _ => None,
        };
        let Some(item) = resolve_dictionary_value(document, Some(value)) else {
            continue;
        };
        if ordinal >= budget {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let location = ObjectLocation {
            object: key,
            offset: key.and_then(|key| document.objects.get(&key).map(|object| object.offset)),
            path: Some(BoundedText::unchecked(format!("root/outlines[{ordinal}]"))),
        };
        append(
            graph,
            (TEXT_STRING, "Title"),
            item.get("Title").and_then(PuaSource::from_object),
            &location,
            objects,
            max_objects,
        )?;
        ordinal = ordinal.saturating_add(1);
        pending.extend(item.get("Next"));
        pending.extend(item.get("First"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write, io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, ProfileReport, Result,
        ValidationFlavour, ValidationOptions, Validator, pdf_text,
    };

    #[rstest]
    #[case(b"\xfe\xff\xe0\x00", true)]
    #[case(b"\xfe\xff\xf8\xff", true)]
    #[case(b"\xfe\xff\xf9\x00", false)]
    #[case(b"\xfe\xff\xdb\x80\xdc\x00", true)]
    #[case(b"\xfe\xff\xdb\xbf\xdf\xfd", true)]
    #[case(b"\xfe\xff\xdb\xbf\xdf\xfe", false)]
    #[case(b"\xfe\xff\xd8\x00\xe0\x00", true)]
    #[case(b"\xef\xbb\xbf\xf4\x80\x80\x80", true)]
    #[case(b"\xef\xbb\xbf\xf4\x8f\xbf\xbd", true)]
    #[case(b"\xef\xbb\xbf\xf4\x8f\xbf\xbe", false)]
    #[case(b"\xef\xbb\xbf\xff\xee\x80\x80", true)]
    #[case(b"\xee\x80\x80", false)]
    #[case(b"\xfe\xff\xdb\x80", false)]
    fn test_should_check_encoded_private_use_ranges(#[case] bytes: &[u8], #[case] expected: bool) {
        assert_eq!(pdf_text::contains_pua(bytes), expected);
    }

    #[rstest]
    #[case("pdfua", 2, "iso32005", "iso-14289-2", [(1, 3), (2, 2), (2, 3)])]
    #[case("wtpdf", 1, "accessibility", "wtpdf-1-0", [(1, 3), (2, 2), (2, 3)])]
    #[case("wtpdf", 1, "reuse", "wtpdf-1-0", [(0, 0), (0, 0), (2, 3)])]
    fn test_should_check_replacement_and_readable_text_at_their_use_sites(
        #[case] family: &str,
        #[case] part: u32,
        #[case] conformance: &str,
        #[case] specification: &str,
        #[case] expected: [(usize, usize); 3],
    ) -> Result<()> {
        let profile = validate(&document()?, family, part, conformance)?;
        for (clause, expected) in ["8-4-3-2", "8-4-3-3", "8-6-1"].into_iter().zip(expected) {
            check_counts(&profile, &format!("{specification}-{clause}"), expected);
        }
        Ok(())
    }

    #[test]
    fn test_should_check_pdfa_actual_text_in_ordinary_streams_and_marked_points() -> Result<()> {
        let profile = validate(&document()?, "pdfa", 4, "none")?;
        check_counts(&profile, "iso-19005-4-6-2-10-8-1", (1, 5));
        Ok(())
    }

    #[test]
    fn test_should_check_shared_form_in_both_views_for_mixed_profiles() -> Result<()> {
        let mut pdf = String::from(
            "%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>\nendobj\n2 0 \
             obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page \
             /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots [6 0 R] /Resources << \
             /XObject << /Used 7 0 R >> >> >>\nendobj\n6 0 obj\n<< /Type /Annot /Subtype /Stamp \
             /Rect [0 0 20 20] /AP << /N 7 0 R >> >>\nendobj\n",
        );
        push_stream(&mut pdf, 4, "", "/Used Do")?;
        push_stream(
            &mut pdf,
            5,
            "/Type /Metadata /Subtype /XML",
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" xmlns:pdfuaid="http://www.aiim.org/pdfua/ns/id/" pdfaid:part="4" pdfaid:rev="2020" pdfuaid:part="2" pdfuaid:rev="2024"/></rdf:RDF></x:xmpmeta>"#,
        )?;
        push_stream(
            &mut pdf,
            7,
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
            "/Span << /ActualText <FEFFE000> /Alt <FEFFE000> >> BDC EMC",
        )?;
        pdf.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        let reports = validate_selection(&pdf, FlavourSelection::Auto { default: None })?;
        assert_eq!(reports.len(), 2);
        let mut checked = 0;
        for profile in &reports {
            match profile.profile.id.as_str() {
                "verapdf-pdfua-2-iso32005" => {
                    check_counts(profile, "iso-14289-2-8-4-3-2", (0, 2));
                    check_counts(profile, "iso-14289-2-8-4-3-3", (0, 2));
                    checked += 1;
                }
                "verapdf-pdfa-4" => {
                    check_counts(profile, "iso-19005-4-6-2-10-8-1", (0, 2));
                    checked += 1;
                }
                _ => {}
            }
        }
        assert_eq!(checked, 2);
        Ok(())
    }

    #[test]
    fn test_should_keep_distinct_property_bindings_for_a_shared_stream() -> Result<()> {
        let mut pdf = String::from(
            "%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 \
             R /MediaBox [0 0 100 100] /Contents 5 0 R /Resources << /Properties << /Text << \
             /ActualText (plain) >> >> >> >>\nendobj\n4 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 100 100] /Contents 5 0 R /Resources << /Properties << /Text << \
             /ActualText <FEFFE000> >> >> >> >>\nendobj\n",
        );
        push_stream(&mut pdf, 5, "", "/Span /Text BDC EMC")?;
        pdf.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        let profile = validate(&pdf, "pdfua", 2, "iso32005")?;
        check_counts(&profile, "iso-14289-2-8-4-3-2", (1, 1));
        Ok(())
    }

    #[rstest]
    #[case("BT /F 10 Tf ET", 0)]
    #[case("BT /F 10 Tf () Tj ET", 1)]
    #[case("BT /F 10 Tf [120] TJ ET", 1)]
    fn test_should_bind_type3_charprocs_at_text_show_operators(
        #[case] text: &str,
        #[case] failures: usize,
    ) -> Result<()> {
        let mut pdf = String::from(
            "%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Font << /F 5 0 R >> >> \
             >>\nendobj\n5 0 obj\n<< /Type /Font /Subtype /Type3 /CharProcs << /A 6 0 R /B 6 0 R \
             >> /FontBBox [0 0 3 3] /FontMatrix [1 0 0 1 0 0] /FirstChar 65 /LastChar 66 /Widths \
             [3 3] /Encoding << /Differences [65 /A /B] >> /Resources <<>> >>\nendobj\n",
        );
        push_stream(&mut pdf, 4, "", text)?;
        push_stream(
            &mut pdf,
            6,
            "",
            "3 0 d0 /Span << /ActualText <FEFFE000> >> BDC EMC",
        )?;
        pdf.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        let profile = validate(&pdf, "pdfa", 4, "none")?;
        check_counts(&profile, "iso-19005-4-6-2-10-8-1", (0, failures));
        Ok(())
    }

    fn check_counts(profile: &ProfileReport, rule: &str, expected: (usize, usize)) {
        let passes = profile
            .passed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule)
            .count();
        let failures = profile
            .failed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule)
            .count();
        assert_eq!((passes, failures), expected, "{rule}");
        assert!(
            !profile
                .unsupported_rules
                .iter()
                .any(|item| item.rule_id.0.as_str() == rule),
            "{rule}"
        );
    }

    fn document() -> Result<String> {
        let mut pdf = String::from(
            "%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Lang (en) /Outlines 8 0 R \
             /StructTreeRoot 6 0 R /Extra << /ActualText 12 0 R /Alt 12 0 R /Title 12 0 R >> \
             >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< \
             /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots [15 0 R] \
             /Resources << /Properties << /Named 14 0 R >> /XObject << /Used 5 0 R >> >> \
             >>\nendobj\n6 0 obj\n<< /Type /StructTreeRoot /K [7 0 R << /Type /Other /S /P \
             /ActualText 12 0 R /Alt 12 0 R /Lang 12 0 R >>] >>\nendobj\n7 0 obj\n<< /Type \
             /StructElem /P 6 0 R /ActualText 12 0 R /Alt (plain) /Lang 12 0 R >>\nendobj\n8 0 \
             obj\n<< /Type /Outlines /First 9 0 R /Last 17 0 R /Title 12 0 R >>\nendobj\n9 0 \
             obj\n<< /Title (plain) /First 10 0 R /Next 11 0 R /Parent 8 0 R >>\nendobj\n10 0 \
             obj\n<< /Title 12 0 R /Parent 9 0 R >>\nendobj\n11 0 obj\n<< /Title 12 0 R /Parent 8 \
             0 R /Prev 17 0 R >>\nendobj\n12 0 obj\n<FEFFE000>\nendobj\n13 0 \
             obj\n<EFBBBFF4808080>\nendobj\n14 0 obj\n<< /ActualText 12 0 R /Alt 13 0 R \
             >>\nendobj\n15 0 obj\n<< /Type /Annot /Subtype /Stamp /Rect [0 0 20 20] /AP << /N 16 \
             0 R >> >>\nendobj\n17 0 obj\n<< /Title 12 0 R /Parent 8 0 R >>\nendobj\n",
        );
        let form = "/Type /XObject /Subtype /Form /BBox [0 0 20 20]";
        push_stream(
            &mut pdf,
            4,
            "",
            "/Artifact << /ActualText <FEFFE000> /Alt (plain) >> BDC /Used Do /Used Do EMC\n/Span \
             /Named BDC EMC\n/Span << /ActualText <FEFFE000> /Alt <FEFFE000> >> DP\n/Span << \
             /ActualText 30 0 R /Alt 7 0 R >> BDC EMC",
        )?;
        push_stream(
            &mut pdf,
            5,
            form,
            "/Span << /ActualText (plain) /Alt <FEFFDB80DC00> >> BDC EMC",
        )?;
        push_stream(
            &mut pdf,
            16,
            form,
            "/Span << /ActualText 12 0 R /Alt 12 0 R >> BDC EMC",
        )?;
        pdf.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        Ok(pdf)
    }

    fn validate(pdf: &str, family: &str, part: u32, conformance: &str) -> Result<ProfileReport> {
        let part = NonZeroU32::new(part).ok_or(ProfileError::UnsupportedSelection)?;
        let flavour = ValidationFlavour::new(family, part, conformance)?;
        validate_selection(pdf, FlavourSelection::Explicit { flavour })?
            .into_iter()
            .next()
            .ok_or_else(|| ProfileError::UnsupportedSelection.into())
    }

    fn validate_selection(pdf: &str, flavour: FlavourSelection) -> Result<Vec<ProfileReport>> {
        let options = ValidationOptions::builder()
            .flavour(flavour)
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        Ok(Validator::new(options)?
            .validate_reader(Cursor::new(pdf.as_bytes()), InputName::memory())?
            .profile_reports)
    }

    fn push_stream(pdf: &mut String, number: u32, entries: &str, text: &str) -> Result<()> {
        write!(
            pdf,
            "{number} 0 obj\n<< {entries} /Length {} >>\nstream\n{text}\nendstream\nendobj\n",
            text.len()
        )
        .map_err(|_| ProfileError::UnsupportedSelection.into())
    }
}
