//! Context-sensitive COS name rule objects.

use super::{GenericModel, ModelObjectRef};
use crate::{CosObject, Dictionary, ModelValue, ParsedDocument, PdfName, Result};

pub(super) const INTENT: &str = "cosRenderingIntent";
pub(super) const FILTER: &str = "cosFilter";
pub(super) const INLINE_FILTER: &str = "cosInlineImageFilter";
pub(super) const BLEND_MODE: &str = "cosBlendMode";
pub(super) const FAMILIES: &[&str] = &[INTENT, FILTER, INLINE_FILTER, BLEND_MODE];

pub(super) fn model<'a>(
    document: &'a ParsedDocument,
    family: &'static str,
    name: &PdfName,
    parameter: Option<&PdfName>,
    ordinal: usize,
    context: &str,
) -> Result<ModelObjectRef<'a>> {
    let mut model = GenericModel::new_value(
        document,
        family,
        ModelValue::from(CosObject::Name(name.clone())),
        ordinal,
        format!("{context}/{family}[{ordinal}]"),
    );
    model.supertypes.clear();
    if let Some(parameter) = parameter {
        model.dictionary.insert(
            PdfName::try_from(String::from("Name"))?,
            CosObject::Name(parameter.clone()),
        );
    }
    Ok(ModelObjectRef::Generic(model))
}

pub(super) fn resolve<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
) -> Option<&'a CosObject> {
    match value? {
        CosObject::Reference(key) => Some(&document.objects.get(key)?.object),
        value => Some(value),
    }
}

pub(super) fn name<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
) -> Option<&'a PdfName> {
    match resolve(document, value)? {
        CosObject::Name(name) => Some(name),
        _ => None,
    }
}

pub(super) fn filter_parameter(
    document: &ParsedDocument,
    stream: &Dictionary,
    filter: &PdfName,
    ordinal: usize,
) -> Result<Option<PdfName>> {
    let params = match resolve(document, stream.get("DecodeParms")) {
        Some(CosObject::Array(params)) => resolve(document, params.get(ordinal)),
        params if ordinal == 0 => params,
        _ => None,
    };
    let params = params.and_then(CosObject::as_dictionary);
    if filter.matches("Crypt")
        && let Some(params) = params
    {
        return Ok(name(document, params.get("Name")).cloned());
    }
    let label = if filter.matches("Crypt") {
        "Identity"
    } else if params.is_some() {
        "Custom"
    } else {
        "Default"
    };
    Ok(Some(PdfName::try_from(label.to_owned())?))
}

pub(super) fn graphics_state_name<'a>(
    document: &'a ParsedDocument,
    value: Option<&'a CosObject>,
    family: &str,
) -> Option<&'a PdfName> {
    match resolve(document, value)? {
        CosObject::Array(values) if family == BLEND_MODE && document.version.major < 2 => {
            values.iter().find_map(|value| name(document, Some(value)))
        }
        CosObject::Name(name) => Some(name),
        _ => None,
    }
}
