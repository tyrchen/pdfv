//! PDF/A signature dictionaries selected by signature fields.

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    ptr,
};

use cms::{cert::CertificateChoices, content_info::ContentInfo, signed_data::SignedData};
use const_oid::db::rfc5911::ID_SIGNED_DATA;
use der::{Decode, Reader, SliceReader};

use super::{
    GenericModel, ModelGraph, ModelObjectRef, catalog_value, push_linked,
    semantic_names::resolve_reference, usize_to_f64,
};
use crate::{
    CosObject, Dictionary, ModelValue, ObjectKey, ParsedDocument, PdfName, PdfString, Result,
    ValidationError,
};

pub(crate) const FIELD_VALUE: &str = "signature";
pub(crate) const REFERENCE: &str = "signatureReference";
pub(crate) const PKCS: &str = "pkcsDataObject";
pub(super) const FAMILIES: &[&str] = &[FIELD_VALUE, REFERENCE, PKCS];
pub(super) const REFERENCE_PROPERTIES: &[&str] = &["permsContainDocMDP", "entries"];
pub(super) const PKCS_PROPERTIES: &[&str] = &["signingCertificatePresent", "SignerInfoCount"];

pub(super) fn append<'a>(
    graph: &ModelGraph<'a>,
    models: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
) -> Result<()> {
    if graph.active_flavour.is_none() || !FAMILIES.iter().any(|family| graph.materializes(family)) {
        return Ok(());
    }
    let document = graph.document;
    let depth = graph.limits.max_object_depth;
    let Some(form) = resolve_reference(document, catalog_value(document, "AcroForm"), depth)?
        .and_then(|(_, value)| value.as_dictionary())
    else {
        return Ok(());
    };
    let mut pending = Vec::new();
    let mut visited = HashSet::new();
    let mut signatures = HashSet::new();
    let mut budget = ScanBudget {
        remaining_objects: max_objects,
        remaining_cms_bytes: graph.limits.max_file_bytes,
        cms: HashMap::new(),
    };
    push_fields(
        document,
        form.get("Fields"),
        depth,
        false,
        &mut pending,
        &mut budget,
    )?;
    while let Some((value, child)) = pending.pop() {
        let Some((field_key, field)) = resolve_reference(document, Some(value), depth)?
            .and_then(|(key, value)| value.as_dictionary().map(|field| (key, field)))
        else {
            continue;
        };
        if child
            && matches!(
                resolve_reference(document, field.get("T"), depth)?,
                None | Some((_, CosObject::Null))
            )
        {
            continue;
        }
        if !visited.insert(ptr::from_ref(field)) {
            continue;
        }
        if field_type(document, field, depth)?.is_some_and(|name| name.matches("Sig"))
            && let Some((key, CosObject::Dictionary(dictionary))) =
                resolve_reference(document, field.get("V"), depth)?
            && signatures.insert(ptr::from_ref(dictionary))
        {
            append_signature(
                graph,
                key,
                field_key,
                dictionary,
                models,
                max_objects,
                &mut budget,
            )?;
        }
        push_fields(
            document,
            field.get("Kids"),
            depth,
            true,
            &mut pending,
            &mut budget,
        )?;
    }
    Ok(())
}

fn push_fields<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
    depth: u32,
    child: bool,
    pending: &mut Vec<(&'a CosObject, bool)>,
    budget: &mut ScanBudget,
) -> Result<()> {
    if let Some((_, CosObject::Array(fields))) = resolve_reference(document, value, depth)? {
        budget.visit(fields.len())?;
        pending.extend(fields.iter().rev().map(|field| (field, child)));
    }
    Ok(())
}

pub(super) fn field_type<'a>(
    document: &'a ParsedDocument,
    mut field: &'a Dictionary,
    depth: u32,
) -> Result<Option<&'a PdfName>> {
    let mut seen = HashSet::new();
    for _ in 0..=depth {
        if !seen.insert(ptr::from_ref(field)) {
            return Ok(None);
        }
        if let Some(value) = field.get("FT")
            && !matches!(value, CosObject::Null)
        {
            return Ok(match resolve_reference(document, Some(value), depth)? {
                Some((_, CosObject::Name(name))) => Some(name),
                _ => None,
            });
        }
        let Some(parent) = resolve_reference(document, field.get("Parent"), depth)?
            .and_then(|(_, value)| value.as_dictionary())
        else {
            return Ok(None);
        };
        field = parent;
    }
    Err(ValidationError::LimitExceeded {
        limit: "max_object_depth",
    }
    .into())
}

fn append_signature<'a>(
    graph: &ModelGraph<'a>,
    key: Option<ObjectKey>,
    field_key: Option<ObjectKey>,
    dictionary: &Dictionary,
    models: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
    budget: &mut ScanBudget,
) -> Result<()> {
    let document = graph.document;
    let depth = graph.limits.max_object_depth;
    let offset = key
        .or(field_key)
        .and_then(|key| document.objects.get(&key))
        .map(|object| object.offset);
    let ordinal = models.len();
    let path = format!("root/catalog[0]/acroForm[0]/signature[{ordinal}]");
    let contents = match resolve_reference(document, dictionary.get("Contents"), depth)? {
        Some((key, CosObject::String(text))) => Some((key, text)),
        _ => None,
    };
    if graph.materializes(FIELD_VALUE) {
        let covered = match (key, contents) {
            (Some(_), Some((_, text))) => {
                byte_range_covers(document, dictionary, offset, text, depth)?
            }
            _ => false,
        };
        let mut model = GenericModel::new(
            document,
            FIELD_VALUE,
            key,
            offset,
            dictionary,
            ordinal,
            &path,
        );
        model.supertypes.clear();
        model.direct_value = Some(ModelValue::Bool(covered));
        push_linked(models, ModelObjectRef::Generic(model), max_objects)?;
    }
    if graph.materializes(PKCS)
        && let Some((contents_key, text)) = contents
    {
        let mut model = GenericModel::new(
            document,
            PKCS,
            None,
            text.file_range.map(|span| span.start).or(offset),
            &Dictionary::default(),
            ordinal,
            format!("{path}/Contents"),
        );
        model.key = contents_key.or(key);
        model.supertypes.clear();
        model.pkcs = Some(budget.pkcs(text)?);
        push_linked(models, ModelObjectRef::Generic(model), max_objects)?;
    }
    if graph.materializes(REFERENCE) {
        append_references(graph, dictionary, &path, models, max_objects, budget)?;
    }
    Ok(())
}

fn append_references<'a>(
    graph: &ModelGraph<'a>,
    signature: &Dictionary,
    path: &str,
    models: &mut Vec<ModelObjectRef<'a>>,
    max_objects: usize,
    budget: &mut ScanBudget,
) -> Result<()> {
    let document = graph.document;
    let depth = graph.limits.max_object_depth;
    let Some((_, CosObject::Array(entries))) =
        resolve_reference(document, signature.get("Reference"), depth)?
    else {
        return Ok(());
    };
    budget.visit(entries.len())?;
    let doc_mdp = matches!(
        resolve_reference(document, catalog_value(document, "Perms"), depth)?,
        Some((_, CosObject::Dictionary(perms)))
            if perms.get("DocMDP").is_some_and(|value| !matches!(value, CosObject::Null))
    );
    for (ordinal, entry) in entries.iter().enumerate() {
        if let Some((key, CosObject::Dictionary(dictionary))) =
            resolve_reference(document, Some(entry), depth)?
        {
            let offset = key
                .and_then(|key| document.objects.get(&key))
                .map(|object| object.offset);
            let mut model = GenericModel::new(
                document,
                REFERENCE,
                key,
                offset,
                dictionary,
                ordinal,
                format!("{path}/Reference[{ordinal}]"),
            );
            model.supertypes.clear();
            model.direct_value = Some(ModelValue::Bool(doc_mdp));
            push_linked(models, ModelObjectRef::Generic(model), max_objects)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ScanBudget {
    remaining_objects: usize,
    remaining_cms_bytes: u64,
    cms: HashMap<*const PdfString, Pkcs>,
}

impl ScanBudget {
    fn visit(&mut self, count: usize) -> Result<()> {
        self.remaining_objects =
            self.remaining_objects
                .checked_sub(count)
                .ok_or(ValidationError::LimitExceeded {
                    limit: "max_objects",
                })?;
        Ok(())
    }

    fn pkcs(&mut self, text: &PdfString) -> Result<Pkcs> {
        match self.cms.entry(ptr::from_ref(text)) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let limit = || ValidationError::LimitExceeded {
                    limit: "signature_cms_bytes",
                };
                let bytes = text.as_bytes();
                let length = u64::try_from(bytes.len()).map_err(|_| limit())?;
                self.remaining_cms_bytes = self
                    .remaining_cms_bytes
                    .checked_sub(length)
                    .ok_or_else(limit)?;
                Ok(*entry.insert(Pkcs::parse(bytes)))
            }
        }
    }
}

fn byte_range_covers(
    document: &ParsedDocument,
    signature: &Dictionary,
    offset: Option<u64>,
    contents: &PdfString,
    depth: u32,
) -> Result<bool> {
    let Some(file_range) = contents.file_range else {
        return Ok(false);
    };
    let Some((_, CosObject::Array(ranges))) =
        resolve_reference(document, signature.get("ByteRange"), depth)?
    else {
        return Ok(false);
    };
    let [
        CosObject::Integer(0),
        CosObject::Integer(first_length),
        CosObject::Integer(second_start),
        CosObject::Integer(second_length),
    ] = ranges.as_slice()
    else {
        return Ok(false);
    };
    let signed_end = u64::try_from(*second_length)
        .ok()
        .and_then(|length| file_range.end.checked_add(length));
    let revision_start = file_range.end.max(offset.unwrap_or(0));
    let revision = document.signed_revision_ends.get(
        document
            .signed_revision_ends
            .partition_point(|end| *end.start() <= revision_start),
    );
    Ok(u64::try_from(*first_length) == Ok(file_range.start)
        && u64::try_from(*second_start) == Ok(file_range.end)
        && revision
            .zip(signed_end)
            .is_some_and(|(valid, end)| valid.contains(&end)))
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pkcs {
    has_x509_certificate: bool,
    signers: usize,
}

impl Pkcs {
    fn parse(data: &[u8]) -> Self {
        fn signed_data(data: &[u8]) -> Option<SignedData> {
            let mut reader = SliceReader::new(data).ok()?;
            let envelope = ContentInfo::decode(&mut reader).ok()?;
            let padding = reader.read_slice(reader.remaining_len()).ok()?;
            if envelope.content_type != ID_SIGNED_DATA || padding.iter().any(|byte| *byte != 0) {
                return None;
            }
            envelope.content.decode_as().ok()
        }
        let Some(data) = signed_data(data) else {
            return Self::default();
        };
        Self {
            has_x509_certificate: data.certificates.is_some_and(|certificates| {
                !certificates.0.is_empty()
                    && certificates
                        .0
                        .iter()
                        .all(|cert| matches!(cert, CertificateChoices::Certificate(_)))
            }),
            signers: data.signer_infos.0.len(),
        }
    }

    pub(super) fn property(self, property: &str) -> Option<Result<ModelValue>> {
        match property {
            "signingCertificatePresent" => Some(Ok(ModelValue::Bool(self.has_x509_certificate))),
            "SignerInfoCount" => Some(usize_to_f64(self.signers).map(ModelValue::Number)),
            _ => None,
        }
    }
}
