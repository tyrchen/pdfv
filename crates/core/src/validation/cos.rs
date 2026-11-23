//! Physical COS primitive traversal and properties for imported rules.

use std::{
    collections::{BTreeSet, btree_map},
    slice,
};

use super::{unknown_property, usize_to_f64};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ParsedDocument, PdfName, PdfString,
    ProfileError, PropertyName, Result, ValidationError, parser::PdfStringSyntax,
    profile::MAX_SAFE_RULE_INTEGER,
};

pub(super) const INTEGER_FAMILY: &str = "cosInteger";
pub(super) const REAL_FAMILY: &str = "cosReal";
pub(super) const NAME_FAMILY: &str = "cosName";
pub(super) const STRING_FAMILY: &str = "cosString";
pub(super) const ARRAY_FAMILY: &str = "cosArray";
pub(super) const DICTIONARY_FAMILY: &str = "cosDictionary";

pub(super) fn is_primitive_family(family: &str) -> bool {
    matches!(
        family,
        INTEGER_FAMILY
            | REAL_FAMILY
            | NAME_FAMILY
            | STRING_FAMILY
            | ARRAY_FAMILY
            | DICTIONARY_FAMILY
    )
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CosPrimitive<'a> {
    Integer(i64),
    Real(f64),
    Name(&'a PdfName),
    String(&'a PdfString),
    Array(usize),
    Dictionary(usize),
}

impl CosPrimitive<'_> {
    pub(super) fn family(self) -> &'static str {
        match self {
            Self::Integer(_) => INTEGER_FAMILY,
            Self::Real(_) => REAL_FAMILY,
            Self::Name(_) => NAME_FAMILY,
            Self::String(_) => STRING_FAMILY,
            Self::Array(_) => ARRAY_FAMILY,
            Self::Dictionary(_) => DICTIONARY_FAMILY,
        }
    }

    pub(super) fn property(self, name: &PropertyName) -> Result<ModelValue> {
        match (self, name.as_str()) {
            (Self::Integer(value), "intValue") => integer_model_value(value),
            (Self::Integer(value), "fitsSigned32") => {
                Ok(ModelValue::Bool(i32::try_from(value).is_ok()))
            }
            (Self::Real(value), "realValue") => Ok(ModelValue::Number(value)),
            (Self::Array(size) | Self::Dictionary(size), "size") => {
                usize_to_f64(size).map(ModelValue::Number)
            }
            (Self::Name(value), "byteLength") => byte_length(value.as_bytes()),
            (Self::String(value), "byteLength") => byte_length(value.as_bytes()),
            (Self::Name(value), "internalRepresentation") => Ok(text_value(value.as_bytes())),
            (Self::String(value), "value") => Ok(text_value(value.as_bytes())),
            (Self::String(value), "isHex" | "containsOnlyHex" | "hexCount") => {
                string_syntax_property(value, name)
            }
            _ => unknown_property(name),
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the magnitude check limits the f64 projection to exactly representable integers"
)]
fn integer_model_value(value: i64) -> Result<ModelValue> {
    if value.unsigned_abs() > MAX_SAFE_RULE_INTEGER {
        return Err(ProfileError::UnsupportedRule {
            reason: BoundedText::unchecked(
                "COS integer exceeds the rule engine's exact numeric range",
            ),
        }
        .into());
    }
    Ok(ModelValue::Number(value as f64))
}

fn byte_length(bytes: &[u8]) -> Result<ModelValue> {
    usize_to_f64(bytes.len()).map(ModelValue::Number)
}

fn text_value(bytes: &[u8]) -> ModelValue {
    ModelValue::String(BoundedText::unchecked(
        String::from_utf8_lossy(bytes).into_owned(),
    ))
}

fn string_syntax_property(value: &PdfString, name: &PropertyName) -> Result<ModelValue> {
    match (value.syntax(), name.as_str()) {
        (PdfStringSyntax::Unknown, _) => Err(ProfileError::UnsupportedRule {
            reason: BoundedText::unchecked("PDF string lexical form is unavailable"),
        }
        .into()),
        (PdfStringSyntax::Hex { digits }, "hexCount") => {
            usize_to_f64(digits).map(ModelValue::Number)
        }
        (PdfStringSyntax::Literal, "hexCount") => Ok(ModelValue::Number(0.0)),
        (PdfStringSyntax::Hex { .. }, _) => Ok(ModelValue::Bool(true)),
        (PdfStringSyntax::Literal, _) => Ok(ModelValue::Bool(false)),
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CosOrigin {
    pub(super) owner: Option<ObjectKey>,
    pub(super) offset: u64,
    pub(super) ordinal: usize,
}

// References are leaves. The object store already visits every referenced
// indirect object once, including objects loaded from object streams.
pub(super) fn walk_primitives<'a>(
    document: &'a ParsedDocument,
    max_nodes: usize,
    mut visit: impl FnMut(CosPrimitive<'a>, CosOrigin) -> Result<()>,
) -> Result<()> {
    let xref_stream_offsets = document.objects.values().filter_map(|object| {
        match &object.object {
            CosObject::Stream(stream)
                if matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("XRef")) =>
            {
                Some(object.offset)
            }
            _ => None,
        }
    }).collect::<BTreeSet<_>>();
    let roots = document
        .objects
        .values()
        .map(|object| {
            (
                CosNode::Value(&object.object),
                Some(object.key),
                object.offset,
            )
        })
        // Xref stream dictionaries also appear as logical trailers.
        .chain(
            document
                .trailers
                .iter()
                .filter(|trailer| !xref_stream_offsets.contains(&trailer.offset))
                .map(|trailer| {
                    (
                        CosNode::Dictionary(&trailer.dictionary),
                        None,
                        trailer.offset,
                    )
                }),
        );
    let mut stack = Vec::new();
    let mut ordinal: usize = 0;
    for (root, owner, offset) in roots {
        let mut current = Some(root);
        while let Some(node) = current {
            if ordinal >= max_nodes {
                return Err(ValidationError::LimitExceeded {
                    limit: "max_objects",
                }
                .into());
            }
            if let Some(primitive) = node.primitive() {
                visit(
                    primitive,
                    CosOrigin {
                        owner,
                        offset,
                        ordinal,
                    },
                )?;
            }
            ordinal = ordinal.saturating_add(1);
            if let Some(children) = node.children() {
                stack.push(children);
            }
            current = next_node(&mut stack);
        }
    }
    Ok(())
}

fn next_node<'a>(stack: &mut Vec<CosChildren<'a>>) -> Option<CosNode<'a>> {
    while let Some(children) = stack.last_mut() {
        if let Some(node) = children.next() {
            return Some(node);
        }
        stack.pop();
    }
    None
}

#[derive(Clone, Copy, Debug)]
enum CosNode<'a> {
    Value(&'a CosObject),
    Name(&'a PdfName),
    Dictionary(&'a Dictionary),
}

impl<'a> CosNode<'a> {
    fn primitive(self) -> Option<CosPrimitive<'a>> {
        match self {
            Self::Value(CosObject::Integer(value)) => Some(CosPrimitive::Integer(*value)),
            Self::Value(CosObject::Real(value)) => Some(CosPrimitive::Real(*value)),
            Self::Value(CosObject::Name(value)) | Self::Name(value) => {
                Some(CosPrimitive::Name(value))
            }
            Self::Value(CosObject::String(value)) => Some(CosPrimitive::String(value)),
            Self::Value(CosObject::Array(values)) => Some(CosPrimitive::Array(values.len())),
            Self::Value(CosObject::Dictionary(values)) | Self::Dictionary(values) => {
                Some(CosPrimitive::Dictionary(values.len()))
            }
            Self::Value(CosObject::Stream(stream)) => {
                Some(CosPrimitive::Dictionary(stream.dictionary.len()))
            }
            Self::Value(_) => None,
        }
    }

    fn children(self) -> Option<CosChildren<'a>> {
        match self {
            Self::Value(CosObject::Array(values)) => Some(CosChildren::Array(values.iter())),
            Self::Dictionary(dictionary) => Some(CosChildren::dictionary(dictionary)),
            Self::Value(value) => value.as_dictionary().map(CosChildren::dictionary),
            Self::Name(_) => None,
        }
    }
}

#[derive(Debug)]
enum CosChildren<'a> {
    Array(slice::Iter<'a, CosObject>),
    Dictionary {
        entries: btree_map::Iter<'a, PdfName, CosObject>,
        pending_value: Option<&'a CosObject>,
    },
}

impl<'a> CosChildren<'a> {
    fn dictionary(dictionary: &'a Dictionary) -> Self {
        Self::Dictionary {
            entries: dictionary.entries(),
            pending_value: None,
        }
    }

    fn next(&mut self) -> Option<CosNode<'a>> {
        match self {
            Self::Array(values) => values.next().map(CosNode::Value),
            Self::Dictionary {
                entries,
                pending_value,
            } => {
                if let Some(value) = pending_value.take() {
                    return Some(CosNode::Value(value));
                }
                let (name, value) = entries.next()?;
                *pending_value = Some(value);
                Some(CosNode::Name(name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, iter::repeat_n, num::NonZeroU32};

    use super::{CosPrimitive, walk_primitives};
    use crate::{
        FeatureSelection, FlavourSelection, InputName, MaxDisplayedFailures, ObjectTypeName,
        Parser, PdfvError, ProfileError, ProfileReport, PropertyName, ResourceLimits, Result,
        ValidationError, ValidationFlavour, ValidationOptions, Validator,
    };

    #[test]
    fn test_should_reject_lossy_custom_integer_projections() -> Result<()> {
        let field = PropertyName::new("intValue")?;
        let value = CosPrimitive::Integer(9_007_199_254_740_993);
        assert!(matches!(
            value.property(&field),
            Err(PdfvError::Profile(ProfileError::UnsupportedRule { .. }))
        ));
        Ok(())
    }

    #[test]
    fn test_should_skip_trailer_view_of_a_physical_xref_stream() -> Result<()> {
        let input = include_bytes!("../../tests/fixtures/xref-stream-object-stream-valid.pdf");
        let document = Parser::default().parse(Cursor::new(input))?;
        let mut root_keys = 0_usize;
        walk_primitives(&document, 256, |primitive, _origin| {
            if matches!(primitive, CosPrimitive::Name(name) if name.matches("Root")) {
                root_keys = root_keys.saturating_add(1);
            }
            Ok(())
        })?;
        assert_eq!(root_keys, 1);
        Ok(())
    }

    #[test]
    fn test_should_reject_primitive_types_as_feature_families() -> Result<()> {
        let options = ValidationOptions::builder()
            .feature_selection(FeatureSelection::Families {
                families: vec![ObjectTypeName::new("cosString")?],
            })
            .build();
        assert!(matches!(
            Validator::new(options),
            Err(PdfvError::Configuration(crate::ConfigError::InvalidValue {
                field: "extract",
                ..
            }))
        ));
        Ok(())
    }

    #[test]
    fn test_should_apply_numeric_rules_to_matching_direct_and_indirect_objects() -> Result<()> {
        let profile = validate_pdfa(
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Values [2147483647 -2147483648 \
               2147483648 -32767.0 32767.0 32767.1 /Name (text) true null 2 0 R 2 0 R] >>\n\
               endobj\n2 0 obj\n-9223372036854775808\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
            NonZeroU32::MIN,
        )?;

        assert_rule_results(&profile, "iso-19005-1-6-1-12-1", 2, 2);
        assert_rule_results(&profile, "iso-19005-1-6-1-12-2", 2, 1);
        Ok(())
    }

    #[test]
    fn test_should_measure_decoded_bytes_of_strings_names_and_dictionary_keys() -> Result<()> {
        let mut pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /".to_vec();
        for _ in 0..128 {
            pdf.extend(b"#ff");
        }
        pdf.extend(b" /");
        pdf.extend(repeat_n(0xff, 127));
        pdf.extend(b" /Strings [(");
        pdf.extend(repeat_n(0xff, 65_532));
        pdf.extend(b"\\377\\\r\n\r\nZ) (");
        pdf.extend(repeat_n(0xff, 65_536));
        pdf.extend(b")] >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");

        let profile = validate_pdfa(&pdf, NonZeroU32::MIN)?;

        assert_rule_results(&profile, "iso-19005-1-6-1-12-3", 1, 1);
        assert_rule_results(&profile, "iso-19005-1-6-1-12-4", 5, 1);
        Ok(())
    }

    #[test]
    fn test_should_distinguish_hex_digit_count_from_literal_and_decoded_bytes() -> Result<()> {
        let part = NonZeroU32::new(2).ok_or(crate::ProfileError::UnsupportedSelection)?;
        let profile = validate_pdfa(
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Strings [<A D> <A C E> (ace)] >>\n\
               endobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
            part,
        )?;

        assert_rule_results(&profile, "iso-19005-2-6-1-6-1", 2, 1);
        assert_rule_results(&profile, "iso-19005-2-6-1-6-2", 3, 0);
        Ok(())
    }

    fn validate_pdfa(pdf: &[u8], part: NonZeroU32) -> Result<ProfileReport> {
        let validator = Validator::new(
            ValidationOptions::builder()
                .flavour(FlavourSelection::Explicit {
                    flavour: ValidationFlavour::new("pdfa", part, "b")?,
                })
                .resource_limits(ResourceLimits {
                    max_name_bytes: 256,
                    ..ResourceLimits::default()
                })
                .max_failed_assertions_per_rule(MaxDisplayedFailures::try_from(8)?)
                .record_passed_assertions(true)
                .build(),
        )?;
        validator
            .validate_reader(Cursor::new(pdf), InputName::memory())?
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

    fn assert_rule_results(profile: &ProfileReport, id: &str, passed: usize, failed: usize) {
        assert_eq!(
            profile
                .passed_assertions
                .iter()
                .filter(|assertion| assertion.rule_id.0.as_str() == id)
                .count(),
            passed,
            "{id}"
        );
        assert_eq!(
            profile
                .failed_assertions
                .iter()
                .filter(|assertion| assertion.rule_id.0.as_str() == id)
                .count(),
            failed,
            "{id}"
        );
        assert!(
            profile
                .unsupported_rules
                .iter()
                .all(|rule| rule.rule_id.0.as_str() != id),
            "{id}"
        );
    }
}
