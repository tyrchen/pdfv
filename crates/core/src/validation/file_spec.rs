//! File specification checks with physical COS and embedded-file name-tree scope.

pub(super) mod association;

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{
    GenericModel, ModelGraph, ModelObjectRef, catalog_dictionary_value,
    cos::{CosOrigin, CosPrimitive},
    object_key_text, push_linked, semantic_names, unknown_property,
};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ParsedDocument,
    PropertyName, Result, ValidationError, pdf_text,
};

pub(crate) const FAMILY: &str = "cosFileSpecification";
pub(crate) const EMBEDDED_FILE: &str = "embeddedAttachment";
pub(super) const PROPERTIES: &[&str] = &[
    "containsEF",
    "containsDesc",
    "presentInEmbeddedFiles",
    "hasNonemptyFileNames",
    "isAssociatedFile",
    "AFRelationship",
    "F",
    "UF",
];

#[derive(Clone, Copy, Debug)]
pub(super) struct Evidence<'a> {
    pub(super) dictionary: &'a Dictionary,
    pub(super) in_name_tree: bool,
    pub(super) associated: bool,
}

#[derive(Debug, Default)]
pub(super) struct Attachments(BTreeSet<ObjectKey>);

impl Attachments {
    pub(super) fn capture(
        &mut self,
        graph: &ModelGraph<'_>,
        object: &ModelObjectRef<'_>,
    ) -> Result<()> {
        if !graph.materializes(EMBEDDED_FILE) {
            return Ok(());
        }
        if let ModelObjectRef::Generic(model) = object
            && let Some(evidence) = model.file_spec
            && let Some((_, CosObject::Dictionary(files))) = resolve_reference(
                graph.document,
                evidence.dictionary.get("EF"),
                graph.limits.max_object_depth,
            )?
        {
            for name in ["F", "UF", "DOS", "Mac", "Unix"] {
                if let Some((Some(key), CosObject::Stream(_))) = resolve_reference(
                    graph.document,
                    files.get(name),
                    graph.limits.max_object_depth,
                )? {
                    self.0.insert(key);
                }
            }
        }
        Ok(())
    }

    pub(super) fn models<'a>(
        &self,
        graph: &ModelGraph<'a>,
        budget: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        if self.0.len() > budget {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let mut models = Vec::with_capacity(self.0.len());
        let document = graph.document;
        for (ordinal, key) in self.0.iter().enumerate() {
            let Some(object) = document.objects.get(key) else {
                continue;
            };
            let CosObject::Stream(stream) = &object.object else {
                continue;
            };
            let valid_mime = matches!(
                resolve_reference(document, stream.dictionary.get("Subtype"), graph.limits.max_object_depth)?,
                Some((_, CosObject::Name(name))) if has_mime_type(name.as_bytes())
            );
            let mut model = GenericModel::new_value(
                document,
                EMBEDDED_FILE,
                ModelValue::Bool(valid_mime),
                ordinal,
                format!("root/{EMBEDDED_FILE}[{}]", object_key_text(*key)),
            );
            model.key = Some(*key);
            model.offset = Some(object.offset);
            model.supertypes.clear();
            models.push(ModelObjectRef::Generic(model));
        }
        Ok(models)
    }
}

pub(super) fn resolve_reference<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
    max_depth: u32,
) -> Result<Option<(Option<ObjectKey>, &'a CosObject)>> {
    let Some(mut value) = value else {
        return Ok(None);
    };
    let mut owner = None;
    let mut aliases = HashSet::new();
    for _ in 0..=max_depth {
        let CosObject::Reference(key) = value else {
            return Ok(Some((owner, value)));
        };
        let Some(object) = document.objects.get(key) else {
            return Ok(None);
        };
        if matches!(object.object, CosObject::Reference(_)) && !aliases.insert(*key) {
            return Ok(None);
        }
        value = &object.object;
        owner = Some(*key);
    }
    Err(ValidationError::LimitExceeded {
        limit: "attachment_reference_depth",
    }
    .into())
}

fn has_mime_type(value: &[u8]) -> bool {
    let mime_token = |token: &[u8]| {
        !token.is_empty()
            && token.iter().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'+' | b'.')
            })
    };
    let mut parts = value.split(|byte| *byte == b'/');
    parts.next().is_some_and(mime_token)
        && parts.next().is_some_and(mime_token)
        && parts.next().is_none()
}

impl Evidence<'_> {
    pub(super) fn property(
        self,
        document: &ParsedDocument,
        name: &PropertyName,
    ) -> Result<ModelValue> {
        let value = match name.as_str() {
            "containsEF" => ModelValue::Bool(self.contains_key("EF")),
            "containsDesc" => ModelValue::Bool(self.contains_key("Desc")),
            "presentInEmbeddedFiles" => ModelValue::Bool(self.in_name_tree),
            "isAssociatedFile" => ModelValue::Bool(self.associated),
            "AFRelationship" => {
                semantic_names::name(document, self.dictionary.get("AFRelationship"))
                    .map_or(ModelValue::Null, |name| {
                        ModelValue::from(CosObject::Name(name.clone()))
                    })
            }
            "hasNonemptyFileNames" => ModelValue::Bool(
                self.has_nonempty_name(document, "F") && self.has_nonempty_name(document, "UF"),
            ),
            "F" | "UF" => {
                match semantic_names::resolve(document, self.dictionary.get(name.as_str())) {
                    Some(CosObject::String(text)) => ModelValue::String(BoundedText::unchecked(
                        pdf_text::decode(text.as_bytes()),
                    )),
                    _ => ModelValue::Null,
                }
            }
            _ => return unknown_property(name),
        };
        Ok(value)
    }

    fn contains_key(self, key: &str) -> bool {
        self.dictionary
            .get(key)
            .is_some_and(|value| !matches!(value, CosObject::Null))
    }

    fn has_nonempty_name(self, document: &ParsedDocument, key: &str) -> bool {
        match semantic_names::resolve(document, self.dictionary.get(key)) {
            Some(CosObject::String(name)) => {
                !matches!(name.as_bytes(), b"" | b"\xfe\xff" | b"\xef\xbb\xbf")
            }
            _ => false,
        }
    }
}

pub(super) fn physical_model<'a>(
    document: &'a ParsedDocument,
    primitive: CosPrimitive<'a>,
    origin: CosOrigin,
) -> Option<ModelObjectRef<'a>> {
    let CosPrimitive::Dictionary(dictionary) = primitive else {
        return None;
    };
    let name = semantic_names::name(document, dictionary.get("Type"))?;
    if !name.matches("Filespec") {
        return None;
    }
    let evidence = Evidence {
        dictionary,
        in_name_tree: false,
        associated: false,
    };
    let location = ObjectLocation {
        object: origin.owner,
        offset: Some(origin.offset),
        path: Some(BoundedText::unchecked(format!(
            "root/{FAMILY}[{}]",
            origin.ordinal
        ))),
    };
    Some(model(document, evidence, location, origin.ordinal))
}

fn model<'a>(
    document: &'a ParsedDocument,
    evidence: Evidence<'a>,
    location: ObjectLocation,
    ordinal: usize,
) -> ModelObjectRef<'a> {
    let mut model = GenericModel::new(
        document,
        FAMILY,
        location.object,
        location.offset,
        &Dictionary::default(),
        ordinal,
        location
            .path
            .map_or_else(String::new, |path| path.as_str().to_owned()),
    );
    model.supertypes.clear();
    model.file_spec = Some(evidence);
    ModelObjectRef::Generic(model)
}

pub(super) fn append_embedded_files<'a>(
    document: &'a ParsedDocument,
    max_objects: usize,
    mut remaining_key_bytes: u64,
    max_reference_depth: u32,
    objects: &mut Vec<ModelObjectRef<'a>>,
) -> Result<()> {
    let Some(root) = catalog_dictionary_value(document, "Names", "EmbeddedFiles") else {
        return Ok(());
    };
    let mut nodes = vec![root];
    let mut visited = HashSet::new();
    let mut expanded_kids = HashSet::new();
    let mut scanned = 0_usize;
    let mut ordinal = 0_usize;
    while let Some(node) = nodes.pop() {
        if let CosObject::Reference(key) = node
            && !visited.insert(*key)
        {
            continue;
        }
        if scanned >= max_objects {
            return Err(ValidationError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        scanned = scanned.saturating_add(1);
        let Some(dictionary) =
            semantic_names::resolve(document, Some(node)).and_then(CosObject::as_dictionary)
        else {
            continue;
        };
        if let Some(CosObject::Array(names)) =
            semantic_names::resolve(document, dictionary.get("Names"))
        {
            for value in name_tree_entries(document, names, &mut remaining_key_bytes)? {
                let Some((key, CosObject::Dictionary(dictionary))) =
                    resolve_reference(document, Some(value), max_reference_depth)?
                else {
                    continue;
                };
                let offset =
                    key.and_then(|key| document.objects.get(&key).map(|object| object.offset));
                let location = ObjectLocation {
                    object: key,
                    offset,
                    path: Some(BoundedText::unchecked(format!(
                        "root/EmbeddedFiles[{ordinal}]"
                    ))),
                };
                let evidence = Evidence {
                    dictionary,
                    in_name_tree: true,
                    associated: false,
                };
                push_linked(
                    objects,
                    model(document, evidence, location, ordinal),
                    max_objects,
                )?;
                ordinal = ordinal.saturating_add(1);
            }
        }
        let kids = dictionary.get("Kids");
        if let Some(CosObject::Reference(key)) = kids
            && !expanded_kids.insert(*key)
        {
            continue;
        }
        if let Some(CosObject::Array(kids)) = semantic_names::resolve(document, kids) {
            if kids.len() > max_objects.saturating_sub(nodes.len()) {
                return Err(ValidationError::LimitExceeded {
                    limit: "max_objects",
                }
                .into());
            }
            nodes.extend(kids.iter().rev());
        }
    }
    Ok(())
}

// A name-tree node maps decoded names to values. Aliases remain independent
// entries, while repeated keys in the same node refer to the last value.
fn name_tree_entries<'a>(
    document: &'a ParsedDocument,
    values: &'a [CosObject],
    remaining_key_bytes: &mut u64,
) -> Result<Vec<&'a CosObject>> {
    let mut indices = HashMap::new();
    let mut entries = Vec::new();
    for pair in values.chunks(2) {
        let Some((key, tail)) = pair.split_first() else {
            continue;
        };
        let value = tail.first();
        let decoded_key = match semantic_names::resolve(document, Some(key)) {
            Some(CosObject::String(text)) => {
                charge_key_bytes(remaining_key_bytes, text.as_bytes().len())?;
                pdf_text::decode(text.as_bytes())
            }
            Some(CosObject::Name(name)) => {
                charge_key_bytes(remaining_key_bytes, name.as_bytes().len())?;
                name.as_bytes().iter().copied().map(char::from).collect()
            }
            _ => continue,
        };
        let next = entries.len();
        let ordinal = *indices.entry(decoded_key).or_insert(next);
        if let Some(existing) = entries.get_mut(ordinal) {
            *existing = value;
        } else {
            entries.push(value);
        }
    }
    Ok(entries.into_iter().flatten().collect())
}

fn charge_key_bytes(remaining_key_bytes: &mut u64, bytes: usize) -> Result<()> {
    let limit = || ValidationError::LimitExceeded {
        limit: "name_tree_key_bytes",
    };
    *remaining_key_bytes = remaining_key_bytes
        .checked_sub(u64::try_from(bytes).map_err(|_| limit())?)
        .ok_or_else(limit)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileError, Result, ValidationFlavour,
        ValidationOptions, Validator,
    };

    const FILE_SPECS: &[u8] = br"%PDF-2.0
1 0 obj
<< /Type /Catalog /Names 2 0 R /Other << /EF <<>> >>
   /Loose << /Type /Filespec /EF <<>> /F /Name /UF (valid) >> >>
endobj
2 0 obj
<< /EmbeddedFiles 3 0 R >>
endobj
3 0 obj
<< /Kids 8 0 R >>
endobj
4 0 obj
<< /Type 7 0 R /EF <<>> /F 6 0 R /UF <FEFF> >>
endobj
5 0 obj
<< /EF <<>> /F (filename) /UF (filename) /Desc () >>
endobj
6 0 obj
(name)
endobj
7 0 obj
/Filespec
endobj
8 0 obj
[9 0 R 10 0 R]
endobj
9 0 obj
<< /Names 11 0 R >>
endobj
10 0 obj
<< /Names [(inline) << /Type /Filespec /EF null /Desc null >>] >>
endobj
11 0 obj
[(dup) 4 0 R <FEFF006400750070> 5 0 R (alias-one) 4 0 R /alias-two 4 0 R 42 4 0 R (tail) 4 0 R (tail)]
endobj
12 0 obj
<< /Type /Filespec /EF null >>
endobj
trailer
<< /Root 1 0 R /Type /Filespec /EF <<>> >>
%%EOF
";

    #[rstest]
    #[case("/Subtype /text#2Fplain", "5 0 R", (1, 0))]
    #[case("/Subtype /application#2Fvnd.test+zip", "6 0 R", (1, 0))]
    #[case("/Subtype /t#C3#A9xt#2Fplain", "5 0 R", (0, 1))]
    #[case("", "5 0 R", (0, 1))]
    #[case("/Type /EmbeddedFile /Subtype /image#2Fpng", "9 0 R", (0, 0))]
    fn test_should_check_mime_on_distinct_ef_linked_streams(
        #[case] dictionary: &str,
        #[case] reference: &str,
        #[case] expected: (usize, usize),
    ) -> Result<()> {
        let pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /AF [2 0 R] /Names << /EmbeddedFiles << /Names \
             [(file) 2 0 R] >> >> >>\nendobj\n2 0 obj\n<< /Type /Filespec /F (file) /UF (file) \
             /AFRelationship /Data /EF 3 0 R >>\nendobj\n3 0 obj\n4 0 R\nendobj\n4 0 obj\n<< /F \
             {reference} /UF {reference} >>\nendobj\n5 0 obj\n<< /Length 1 {dictionary} \
             >>\nstream\nx\nendstream\nendobj\n6 0 obj\n5 0 R\nendobj\ntrailer\n<< /Root 1 0 R \
             >>\n%%EOF\n"
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
        let matches_rule =
            |assertion: &&crate::Assertion| assertion.rule_id.0.as_str() == "iso-19005-3-6-8-1";
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
            expected
        );
        Ok(())
    }

    #[rstest]
    #[case("pdfua", 1, "none", "iso-14289-1-7-11-1", 2, 2)]
    #[case("pdfua", 2, "iso32005", "iso-14289-2-8-14-1-1", 3, 0)]
    #[case("wtpdf", 1, "accessibility", "wtpdf-1-0-8-14-1-1", 3, 0)]
    fn test_should_check_embedded_file_occurrences_and_physical_file_specs(
        #[case] family: &str,
        #[case] part: u32,
        #[case] conformance: &str,
        #[case] rule: &str,
        #[case] named_failures: usize,
        #[case] physical_failures: usize,
    ) -> Result<()> {
        let part = NonZeroU32::new(part).ok_or(ProfileError::UnsupportedSelection)?;
        let flavour = ValidationFlavour::new(family, part, conformance)?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        let report = Validator::new(options)?
            .validate_reader(Cursor::new(FILE_SPECS), InputName::memory())?;
        let profile = report
            .profile_reports
            .first()
            .ok_or(ProfileError::UnsupportedSelection)?;
        assert!(
            !profile
                .unsupported_rules
                .iter()
                .any(|result| result.rule_id.0.as_str() == rule)
        );
        for (context_prefix, failed) in [
            ("root/EmbeddedFiles[", named_failures),
            ("root/cosFileSpecification[", physical_failures),
        ] {
            let count = |assertions: &[crate::Assertion]| {
                assertions
                    .iter()
                    .filter(|assertion| {
                        assertion.rule_id.0.as_str() == rule
                            && assertion
                                .object_context
                                .as_ref()
                                .is_some_and(|context| context.as_str().starts_with(context_prefix))
                    })
                    .count()
            };
            assert_eq!(
                count(&profile.failed_assertions),
                failed,
                "{context_prefix}"
            );
            assert_eq!(
                count(&profile.passed_assertions),
                4_usize.saturating_sub(failed),
                "{context_prefix}"
            );
        }
        Ok(())
    }
}
