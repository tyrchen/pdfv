//! Annotation applicability and rendition media-clip evidence.

use std::{collections::HashSet, iter::successors, ptr};

use super::{
    AnnotationModel, GenericModel, ModelGraph, ModelObjectRef, array_values, cos_integer,
    inherited_page_value, number_from_cos, optional_string_model_value, push_linked,
    semantic_names,
};
use crate::{
    CosObject, Dictionary, ModelValue, ObjectKey, ObjectTypeName, ParsedDocument, PropertyName,
    Result, ValidationError, pdf_text,
};

pub(crate) const MEDIA_CLIP: &str = "mediaClipData";
pub(super) const MEDIA_PROPERTIES: &[&str] = &["CT", "Alt", "hasCorrectAlt"];

const SUBTYPES: &[(&str, &str, &str)] = &[
    ("PDWidgetAnnot", "Widget", "widgetAnnotation"),
    ("PDLinkAnnot", "Link", "linkAnnotation"),
    ("PDTrapNetAnnot", "TrapNet", "trapNetAnnotation"),
    ("PDPrinterMarkAnnot", "PrinterMark", "printerMarkAnnotation"),
];

pub(crate) fn source_family(source: &str) -> Option<&'static str> {
    SUBTYPES
        .iter()
        .find_map(|&(tag, _, family)| (source == tag).then_some(family))
}

pub(super) fn families() -> impl Iterator<Item = &'static str> {
    SUBTYPES.iter().map(|&(_, _, family)| family)
}

pub(super) fn supertypes(
    document: &ParsedDocument,
    dictionary: &Dictionary,
) -> Vec<ObjectTypeName> {
    let mut types = vec![ObjectTypeName::unchecked("object")];
    if let Some(name) = semantic_names::name(document, dictionary.get("Subtype"))
        && let Some(&(_, _, family)) = SUBTYPES
            .iter()
            .find(|(_, subtype, _)| name.matches(subtype))
    {
        types.push(ObjectTypeName::unchecked(family));
    }
    types
}

#[derive(Clone, Debug)]
pub(super) struct StructureParent<'a> {
    dictionary: &'a Dictionary,
    standard_role: Option<String>,
    artifact: bool,
}

impl<'a> StructureParent<'a> {
    pub(super) fn read(graph: &ModelGraph<'a>, annotation: &Dictionary) -> Result<Option<Self>> {
        let document = graph.document;
        if semantic_names::resolve(document, annotation.get("StructParent"))
            .and_then(cos_integer)
            .is_none()
        {
            return Ok(None);
        }
        let structure = graph.accessibility_graph()?;
        let Some(key) = structure.content_parent(document, annotation, None)? else {
            return Ok(None);
        };
        let Some(dictionary) = document
            .objects
            .get(&key)
            .and_then(|object| object.object.as_dictionary())
        else {
            return Ok(None);
        };
        let node = structure.node_from_key(key);
        let standard_role = node
            .filter(|node| !node.non_standard_role)
            .map(|node| node.normalized_role.clone());
        let artifact = successors(node, |node| {
            node.parent.and_then(|parent| structure.node(parent))
        })
        .take(
            usize::try_from(graph.limits.max_structure_depth)
                .unwrap_or(usize::MAX)
                .saturating_add(1),
        )
        .any(|ancestor| ancestor.normalized_role == "Artifact");
        Ok(Some(Self {
            dictionary,
            standard_role,
            artifact,
        }))
    }

    pub(super) fn value(&self, document: &ParsedDocument, property: &str) -> ModelValue {
        match property {
            "structParentType" => semantic_names::name(document, self.dictionary.get("S"))
                .map_or(ModelValue::Null, |name| {
                    ModelValue::from(CosObject::Name(name.clone()))
                }),
            "structParentStandardType" => optional_string_model_value(self.standard_role.clone()),
            "Alt" => text_value(document, self.dictionary.get("Alt")),
            "containsLang" => ModelValue::Bool(matches!(
                semantic_names::resolve(document, self.dictionary.get("Lang")),
                Some(CosObject::String(_))
            )),
            "isArtifact" => ModelValue::Bool(self.artifact),
            _ => ModelValue::Null,
        }
    }
}

pub(super) fn text_value(document: &ParsedDocument, value: Option<&CosObject>) -> ModelValue {
    match semantic_names::resolve(document, value) {
        Some(CosObject::String(text)) => {
            optional_string_model_value(Some(pdf_text::decode(text.as_bytes())))
        }
        _ => ModelValue::Null,
    }
}

#[derive(Clone, Copy, Debug)]
struct Rectangle {
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
}

impl Rectangle {
    fn read(document: &ParsedDocument, value: Option<&CosObject>) -> Option<Self> {
        let CosObject::Array(coordinates) = semantic_names::resolve(document, value)? else {
            return None;
        };
        let [left, bottom, right, top] = coordinates.as_slice() else {
            return None;
        };
        let coordinate = |value| number_from_cos(semantic_names::resolve(document, Some(value))?);
        let (left, bottom, right, top) = (
            coordinate(left)?,
            coordinate(bottom)?,
            coordinate(right)?,
            coordinate(top)?,
        );
        Some(Self {
            left: left.min(right),
            bottom: bottom.min(top),
            right: left.max(right),
            top: bottom.max(top),
        })
    }

    fn intersection(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            bottom: self.bottom.max(other.bottom),
            right: self.right.min(other.right),
            top: self.top.min(other.top),
        }
    }

    fn disjoint(self, other: Self) -> bool {
        self.left >= other.right
            || self.right <= other.left
            || self.bottom >= other.top
            || self.top <= other.bottom
    }
}

pub(super) fn outside_crop(annotation: &AnnotationModel<'_>) -> Result<ModelValue> {
    let document = annotation.document;
    let Some(rect) = Rectangle::read(document, annotation.dictionary.get("Rect")) else {
        return Ok(ModelValue::Null);
    };
    let page_box = |name| {
        inherited_page_value(document, annotation.page_key, name, annotation.limits)
            .map(|value| Rectangle::read(document, value))
    };
    let media = page_box("MediaBox")?;
    let crop = match (page_box("CropBox")?, media) {
        (Some(crop), Some(media)) => Some(crop.intersection(media)),
        (crop, media) => crop.or(media),
    };
    Ok(crop.map_or(ModelValue::Null, |crop| {
        ModelValue::Bool(crop.disjoint(rect))
    }))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MediaNode {
    Rendition,
    Clip,
}

pub(super) fn append_media_clips<'a>(
    action: &GenericModel<'_>,
    graph: &ModelGraph<'a>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    if graph.active_flavour.is_none()
        || !graph.materializes(MEDIA_CLIP)
        || !semantic_names::name(document, action.dictionary.get("S"))
            .is_some_and(|name| name.matches("Rendition"))
    {
        return Ok(());
    }
    let Some(rendition) = action.dictionary.get("R") else {
        return Ok(());
    };
    let mut pending = vec![(MediaNode::Rendition, rendition)];
    let mut visited = HashSet::new();
    while let Some((kind, value)) = pending.pop() {
        // Reusing a rendition from several actions still consumes traversal work.
        let remaining = graph
            .media_clip_nodes_remaining
            .get()
            .checked_sub(1)
            .ok_or(ValidationError::LimitExceeded {
                limit: "max_objects",
            })?;
        graph.media_clip_nodes_remaining.set(remaining);
        let Some((key, CosObject::Dictionary(dictionary))) = semantic_names::resolve_reference(
            document,
            Some(value),
            graph.limits.max_object_depth,
        )?
        else {
            continue;
        };
        if !visited.insert((kind, ptr::from_ref(dictionary))) {
            continue;
        }
        let name = semantic_names::name(document, dictionary.get("S"));
        match kind {
            MediaNode::Rendition if name.is_some_and(|name| name.matches("SR")) => {
                let children = semantic_names::resolve(document, dictionary.get("R"));
                for child in array_values(children) {
                    if pending.len().saturating_add(visited.len()) >= max {
                        return Err(ValidationError::LimitExceeded {
                            limit: "max_objects",
                        }
                        .into());
                    }
                    pending.push((MediaNode::Rendition, child));
                }
            }
            MediaNode::Rendition if name.is_none_or(|name| name.matches("MR")) => {
                pending.extend(dictionary.get("C").map(|value| (MediaNode::Clip, value)));
            }
            MediaNode::Clip if name.is_some_and(|name| name.matches("MCS")) => {
                pending.extend(dictionary.get("D").map(|value| (MediaNode::Clip, value)));
            }
            MediaNode::Clip if name.is_none_or(|name| name.matches("MCD")) => {
                let ordinal = objects.len();
                let offset = key.and_then(|key: ObjectKey| {
                    document.objects.get(&key).map(|object| object.offset)
                });
                push_linked(
                    objects,
                    ModelObjectRef::Generic(GenericModel::new(
                        document,
                        MEDIA_CLIP,
                        key,
                        offset,
                        dictionary,
                        ordinal,
                        format!("{}/mediaClip[{ordinal}]", action.context),
                    )),
                    max,
                )?;
            }
            _ => {}
        }
        if pending.len().saturating_add(visited.len()) > max {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
    }
    Ok(())
}

pub(super) fn media_property(
    model: &GenericModel<'_>,
    property: &PropertyName,
) -> Option<ModelValue> {
    let document = model.document;
    let value = model.dictionary.get(property.as_str());
    Some(match property.as_str() {
        "CT" => text_value(document, value),
        "Alt" => semantic_names::resolve(document, value)
            .cloned()
            .map_or(ModelValue::Null, ModelValue::from),
        "hasCorrectAlt" => {
            let valid = matches!(semantic_names::resolve(document, model.dictionary.get("Alt")),
            Some(CosObject::Array(values)) if values.len().is_multiple_of(2)
            && values.iter().enumerate().all(|(index, value)| matches!(
                semantic_names::resolve(document, Some(value)),
                    Some(CosObject::String(text)) if index.is_multiple_of(2) || !pdf_text::decode(text.as_bytes()).is_empty()
            )));
            ModelValue::Bool(valid)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, ProfileReport, Result, ValidationError, ValidationFlavour,
        ValidationOptions, Validator,
    };

    #[rstest]
    #[case("/Stamp", "/Rect [10 10 20 20]", (0, 1), (0, 1), (0, 0))]
    #[case("/Stamp", "/Rect [10 10 20 20] /F 8 0 R", (1, 0), (1, 0), (0, 0))]
    #[case("/Stamp", "/Rect [65 10 75 20]", (1, 0), (1, 0), (0, 0))]
    #[case("/Stamp", "/Rect [20 20 20 20]", (0, 1), (0, 1), (0, 0))]
    #[case("/Stamp", "/Rect [25 25 10 10]", (0, 1), (0, 1), (0, 0))]
    #[case("/Link", "/Rect [10 10 20 20]", (1, 0), (0, 1), (0, 1))]
    fn test_should_scope_annotation_rules_by_subtype_and_viewport(
        #[case] subtype: &str,
        #[case] entries: &str,
        #[case] tag: (usize, usize),
        #[case] description: (usize, usize),
        #[case] link_tag: (usize, usize),
    ) -> Result<()> {
        let pdf = format!(
            "4 0 obj
<< /Type /Annot /Subtype {subtype} {entries} >>
endobj
8 0 obj
2
endobj
"
        );
        let profile = validate("[4 0 R]", "", &pdf)?;
        assert_rule(&profile, "7-18-1-1", tag);
        assert_rule(&profile, "7-18-1-2", description);
        assert_rule(&profile, "7-18-5-1", link_tag);
        assert_rule(&profile, "7-18-1-3", (0, 0));
        assert_rule(&profile, "7-18-2-1", (0, 0));
        Ok(())
    }

    #[test]
    fn test_should_use_role_map_and_alt_from_the_enclosing_structure_element() -> Result<()> {
        let objects = "4 0 obj
<< /Type /Annot /Subtype /Stamp /Rect [10 10 20 20] /StructParent 0 >>
endobj
5 0 obj
<< /Type /StructTreeRoot /K 6 0 R /RoleMap << /NoteStamp /Annot >>
/ParentTree << /Nums [0 6 0 R] >> >>
endobj
6 0 obj
<< /Type /StructElem /S /NoteStamp /Alt (Stamped) /P 5 0 R
/K << /Type /OBJR /Obj 4 0 R /Pg 3 0 R >> >>
endobj
";
        let profile = validate("[4 0 R]", "/StructTreeRoot 5 0 R", objects)?;
        assert_rule(&profile, "7-18-1-1", (1, 0));
        assert_rule(&profile, "7-18-1-2", (1, 0));
        Ok(())
    }

    #[rstest]
    #[case("/Rendition", "/CT (video/mp4) /Alt [() (Description)]", (1, 0), (1, 0))]
    #[case("/Rendition", "/CT /Video /Alt [() <FEFF>]", (0, 1), (0, 1))]
    #[case("/URI", "", (0, 0), (0, 0))]
    fn test_should_check_only_the_reachable_media_clip_data(
        #[case] action: &str,
        #[case] keys: &str,
        #[case] content_type: (usize, usize),
        #[case] alt: (usize, usize),
    ) -> Result<()> {
        let objects = format!(
            "4 0 obj
<< /Type /Annot /Subtype /Screen /Rect [10 10 20 20] /A 5 0 R >>
endobj
5 0 obj
<< /S {action} /R << /S /SR /R [<< /S /MR /C << /S /MCS /D 6 0 R >> >>] >> >>
endobj
6 0 obj
<< /Type /MediaClip /S /MCD {keys} >>
endobj
7 0 obj
<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 0 >>
stream
endstream
endobj
8 0 obj
<< /Filter /ASCIIHexDecode /V 82 /Length 1 >>
stream
>
endstream
endobj
"
        );
        let profile = validate("[4 0 R]", "", &objects)?;
        assert_rule(&profile, "7-18-6-2-1", content_type);
        assert_rule(&profile, "7-18-6-2-2", alt);
        assert_rule(&profile, "7-16-1", (0, 0));
        Ok(())
    }

    fn validate(annotations: &str, catalog: &str, objects: &str) -> Result<ProfileReport> {
        let pdf = format!(
            "%PDF-1.7
1 0 obj
<< /Type /Catalog /Lang (en) /Pages 2 0 R {catalog} >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] /CropBox [0 0 60 60] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Annots {annotations} /Tabs /S >>
endobj
{objects}trailer
<< /Root 1 0 R >>
%%EOF
"
        );
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit {
                flavour: ValidationFlavour::new("pdfua", NonZeroU32::MIN, "none")?,
            })
            .record_passed_assertions(true)
            .build();
        Validator::new(options)?
            .validate_reader(Cursor::new(pdf.into_bytes()), InputName::memory())?
            .profile_reports
            .into_iter()
            .next()
            .ok_or_else(|| {
                ValidationError::SubsystemUnavailable {
                    subsystem: "testProfile",
                }
                .into()
            })
    }

    fn assert_rule(profile: &ProfileReport, clause: &str, counts: (usize, usize)) {
        let rule = format!("iso-14289-1-{clause}");
        let passed = profile
            .passed_assertions
            .iter()
            .filter(|assertion| assertion.rule_id.0.as_str() == rule)
            .count();
        let failed = profile
            .failed_assertions
            .iter()
            .filter(|assertion| assertion.rule_id.0.as_str() == rule)
            .count();
        assert_eq!((passed, failed), counts, "{rule}");
        assert!(
            profile
                .unsupported_rules
                .iter()
                .all(|entry| entry.rule_id.0.as_str() != rule),
            "{rule}"
        );
    }
}
