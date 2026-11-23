//! Glyph-width and PDF/A-1 `CIDSet` evidence from actual text-show operators.

mod cff;
mod sfnt;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    ContentStreamModel, ContentStreamSummary, EffectiveResources, GenericModel, ModelGraph,
    ModelObjectRef, OperatorFact, ResourceFamily, array_values, cos_integer,
    descendant_font_dictionary, descendant_font_value, font_descriptor_dictionary, font_name,
    font_program_stream, push_linked, resolve_named_dictionary, resource_value, semantic_names,
    stream_from_value, unknown_property,
};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ParsedDocument,
    PdfName, PropertyName, ResourceLimits, Result, ValidationError, content::TextOperand,
};

pub(crate) const WIDTH_FAMILY: &str = "glyphWidth";
pub(crate) const CID_SET_FAMILY: &str = "usedCidSet";

#[derive(Clone, Copy, Debug)]
enum Measurement {
    Known(f64),
    Absent,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Coverage {
    Verified,
    Missing,
    Unavailable,
}

impl Coverage {
    fn include(self, other: Self) -> Self {
        match (self, other) {
            (Self::Missing, _) | (_, Self::Missing) => Self::Missing,
            (Self::Unavailable, _) | (_, Self::Unavailable) => Self::Unavailable,
            _ => Self::Verified,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Evidence {
    Width(WidthEvidence),
    CidSet(CidSet),
    UnresolvedCidFont,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WidthEvidence {
    mode: Option<u8>,
    program: Measurement,
    dictionary: Measurement,
}

#[derive(Clone, Debug)]
pub(super) struct CidSet {
    name: Option<String>,
    present: bool,
    coverage: Coverage,
}

impl Evidence {
    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match (self, name.as_str()) {
            (Self::Width(evidence), "renderingMode") => evidence.mode.map_or_else(
                || unknown_property(name),
                |mode| Ok(ModelValue::Number(f64::from(mode))),
            ),
            (Self::Width(evidence), "widthFromFontProgram") => measurement(evidence.program, name),
            (Self::Width(evidence), "widthFromDictionary") => {
                measurement(evidence.dictionary, name)
            }
            (Self::CidSet(evidence), "fontName") => {
                Ok(super::optional_string_model_value(evidence.name.clone()))
            }
            (Self::CidSet(evidence), "containsCIDSet") => Ok(ModelValue::Bool(evidence.present)),
            (Self::CidSet(evidence), "cidSetListsAllGlyphs") => match evidence.coverage {
                Coverage::Verified => Ok(ModelValue::Bool(true)),
                Coverage::Missing => Ok(ModelValue::Bool(false)),
                Coverage::Unavailable => unknown_property(name),
            },
            _ => unknown_property(name),
        }
    }
}

fn measurement(value: Measurement, name: &PropertyName) -> Result<ModelValue> {
    match value {
        Measurement::Known(number) => Ok(ModelValue::Number(number)),
        Measurement::Absent => Ok(ModelValue::Null),
        Measurement::Unavailable => unknown_property(name),
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum FontId {
    Indirect(ObjectKey),
    Direct(ObjectKey, u64),
}

#[derive(Clone, Debug)]
struct FontSelection {
    id: FontId,
    direct: Option<Arc<Dictionary>>,
}

impl FontSelection {
    fn dictionary<'a>(&'a self, document: &'a ParsedDocument) -> Option<&'a Dictionary> {
        match self.id {
            FontId::Indirect(key) => document.objects.get(&key)?.object.as_dictionary(),
            FontId::Direct(..) => self.direct.as_deref(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct TextState {
    font: Option<FontSelection>,
    mode: Option<u8>,
}

impl TextState {
    pub(super) fn initial() -> Self {
        Self {
            font: None,
            mode: Some(0),
        }
    }

    pub(super) fn identity(&self) -> String {
        format!(
            "{:?}:{:?}",
            self.font.as_ref().map(|font| font.id),
            self.mode
        )
    }
}

#[derive(Debug, Default)]
pub(super) struct CidAudit {
    fonts: BTreeMap<FontId, (CidSet, String)>,
    unresolved: Option<String>,
    type3_procedures: u64,
}

impl CidAudit {
    fn record(
        &mut self,
        id: FontId,
        document: &ParsedDocument,
        font: &Dictionary,
        context: &str,
    ) -> Option<&mut CidSet> {
        let cid = descendant_font_dictionary(document, font).unwrap_or(font);
        if !matches!(semantic_names::name(document, cid.get("Subtype")), Some(name) if name.matches("CIDFontType0") || name.matches("CIDFontType2"))
        {
            return None;
        }
        let id = cid_font_id(document, font, id);
        if let std::collections::btree_map::Entry::Vacant(entry) = self.fonts.entry(id) {
            let cid_set = font_descriptor_dictionary(document, cid)
                .and_then(|descriptor| descriptor.get("CIDSet"));
            let present = stream_from_value(document, cid_set).is_some();
            let coverage = if present {
                Coverage::Verified
            } else {
                Coverage::Missing
            };
            entry.insert((
                CidSet {
                    name: font_name(document, cid),
                    present,
                    coverage,
                },
                context.to_owned(),
            ));
        }
        self.fonts.get_mut(&id).map(|(record, _)| record)
    }

    pub(super) fn models<'a>(
        &self,
        document: &'a ParsedDocument,
        max: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut models = Vec::new();
        for (ordinal, (record, context)) in self.fonts.values().enumerate() {
            push_linked(
                &mut models,
                model(
                    document,
                    CID_SET_FAMILY,
                    Evidence::CidSet(record.clone()),
                    ordinal,
                    context,
                ),
                max,
            )?;
        }
        if let Some(context) = &self.unresolved {
            let ordinal = models.len();
            push_linked(
                &mut models,
                model(
                    document,
                    CID_SET_FAMILY,
                    Evidence::UnresolvedCidFont,
                    ordinal,
                    context,
                ),
                max,
            )?;
        }
        Ok(models)
    }
}

fn cid_font_id(document: &ParsedDocument, font: &Dictionary, fallback: FontId) -> FontId {
    if let Some(CosObject::Reference(key)) = descendant_font_value(document, font) {
        FontId::Indirect(*key)
    } else {
        fallback
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample<'a> {
    text: Option<&'a TextOperand>,
    mode: Option<u8>,
    location: &'a ObjectLocation,
}

#[derive(Debug, Default)]
struct TextUses<'a> {
    fonts: BTreeMap<FontId, (FontSelection, Vec<Sample<'a>>)>,
    unresolved: Option<Sample<'a>>,
    invocations: BTreeMap<u64, TextState>,
}

pub(super) fn inspect<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    summary: &ContentStreamSummary,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<BTreeMap<u64, TextState>> {
    if content.is_form && content.initial_text_state.is_none() {
        return Ok(BTreeMap::new());
    }
    let resources = content.effective_resources(graph.document)?;
    let uses = collect_uses(content, graph.document, &resources, summary, graph.limits)?;
    let mut cid_audit = graph.cid_audit_mut()?;
    if graph.materializes(CID_SET_FAMILY) && uses.unresolved.is_some() {
        cid_audit
            .unresolved
            .get_or_insert_with(|| content.context_prefix.clone());
    }
    if graph.materializes(WIDTH_FAMILY)
        && let Some(sample) = uses.unresolved
    {
        push_width(
            objects,
            graph.document,
            sample,
            Measurement::Unavailable,
            Measurement::Unavailable,
            content,
            max,
        )?;
    }
    for (font, samples) in uses.fonts.values() {
        if let Some(dictionary) = font.dictionary(graph.document) {
            inspect_font(
                content,
                graph,
                font.id,
                dictionary,
                samples,
                &mut cid_audit,
                objects,
                max,
            )?;
        } else if graph.materializes(WIDTH_FAMILY)
            && let Some(sample) = samples.first().copied()
        {
            push_width(
                objects,
                graph.document,
                sample,
                Measurement::Unavailable,
                Measurement::Unavailable,
                content,
                max,
            )?;
        }
    }
    Ok(uses.invocations)
}

fn collect_uses<'a>(
    content: &ContentStreamModel<'_>,
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    summary: &'a ContentStreamSummary,
    limits: &ResourceLimits,
) -> Result<TextUses<'a>> {
    let mut uses = TextUses::default();
    let mut selections = FontSelections::new(summary);
    let mut state = content
        .initial_text_state
        .clone()
        .unwrap_or_else(TextState::initial);
    let mut saved = Vec::new();
    for fact in &summary.facts {
        match fact {
            OperatorFact::GraphicsState { op, .. } if op.as_str() == "q" => {
                if u32::try_from(saved.len()).unwrap_or(u32::MAX) >= limits.max_graphics_state_depth
                {
                    return Err(ValidationError::LimitExceeded {
                        limit: "max_graphics_state_depth",
                    }
                    .into());
                }
                saved.push(state.clone());
            }
            OperatorFact::GraphicsState { op, .. } if op.as_str() == "Q" => {
                if let Some(restored) = saved.pop() {
                    state = restored;
                }
            }
            OperatorFact::GraphicsState { op, location } if op.as_str() == "gs" => {
                selections
                    .change(location, document, resources, content.key)
                    .apply(&mut state);
            }
            OperatorFact::TextState { op, location, .. } if op.as_str() == "Tf" => {
                selections
                    .change(location, document, resources, content.key)
                    .apply(&mut state);
            }
            OperatorFact::TextState {
                op, rendering_mode, ..
            } if op.as_str() == "Tr" => {
                state.mode = rendering_mode
                    .and_then(|mode| u8::try_from(mode).ok())
                    .filter(|mode| *mode <= 7);
            }
            OperatorFact::TextShow { location, .. } => {
                let text = location
                    .offset
                    .and_then(|offset| summary.text_operands.get(&offset));
                if text.is_some_and(|text| text.0.iter().all(|string| string.as_bytes().is_empty()))
                {
                    continue;
                }
                let sample = Sample {
                    text,
                    mode: state.mode,
                    location,
                };
                if let Some(font) = &state.font {
                    uses.fonts
                        .entry(font.id)
                        .or_insert_with(|| (font.clone(), Vec::new()))
                        .1
                        .push(sample);
                } else if uses
                    .unresolved
                    .is_none_or(|previous| previous.mode == Some(3))
                {
                    uses.unresolved = Some(sample);
                }
            }
            OperatorFact::XObjectInvoke { location, .. } => {
                if let Some(offset) = location.offset {
                    uses.invocations.insert(offset, state.clone());
                }
            }
            OperatorFact::Color { op, location, .. } if matches!(op.as_str(), "SCN" | "scn") => {
                if let Some(offset) = location.offset {
                    uses.invocations.insert(offset, state.clone());
                }
            }
            _ => {}
        }
    }
    Ok(uses)
}

#[derive(Clone, Debug)]
enum FontChange {
    Unchanged,
    Selected(Option<FontSelection>),
}

impl FontChange {
    fn apply(self, state: &mut TextState) {
        if let Self::Selected(selection) = self {
            state.font = selection;
        }
    }
}

#[derive(Debug)]
struct FontSelections<'a> {
    at_offset: BTreeMap<u64, &'a crate::content::ResourceUse>,
    by_name: BTreeMap<(&'static str, &'a PdfName), FontChange>,
}

impl<'a> FontSelections<'a> {
    fn new(summary: &'a ContentStreamSummary) -> Self {
        let at_offset = summary
            .resource_uses
            .iter()
            .filter(|usage| {
                matches!(
                    usage.family,
                    ResourceFamily::Font | ResourceFamily::ExtGState
                )
            })
            .filter_map(|usage| Some((usage.location.offset?, usage)))
            .collect();
        Self {
            at_offset,
            by_name: BTreeMap::new(),
        }
    }

    fn change(
        &mut self,
        location: &ObjectLocation,
        document: &ParsedDocument,
        resources: &EffectiveResources<'_>,
        owner: ObjectKey,
    ) -> FontChange {
        let Some(offset) = location.offset else {
            return FontChange::Selected(None);
        };
        let Some(usage) = self.at_offset.get(&offset) else {
            return FontChange::Selected(None);
        };
        self.by_name
            .entry((usage.family.as_str(), &usage.name))
            .or_insert_with(|| {
                let Some(value) = lookup(document, resources, usage.family, &usage.name) else {
                    return FontChange::Selected(None);
                };
                let font = if usage.family == ResourceFamily::ExtGState {
                    let Some(dictionary) = semantic_names::resolve(document, Some(value))
                        .and_then(CosObject::as_dictionary)
                    else {
                        return FontChange::Selected(None);
                    };
                    if dictionary.get("Font").is_none() {
                        return FontChange::Unchanged;
                    }
                    array_values(semantic_names::resolve(document, dictionary.get("Font"))).next()
                } else {
                    Some(value)
                };
                FontChange::Selected(
                    font.and_then(|font| font_selection(document, font, owner, offset)),
                )
            })
            .clone()
    }
}

fn lookup<'a>(
    document: &'a ParsedDocument,
    resources: &EffectiveResources<'a>,
    family: ResourceFamily,
    name: &PdfName,
) -> Option<&'a CosObject> {
    resources
        .dictionaries
        .iter()
        .rev()
        .find_map(|dictionary| resource_value(document, dictionary, family, name))
}

fn font_selection(
    document: &ParsedDocument,
    value: &CosObject,
    content: ObjectKey,
    offset: u64,
) -> Option<FontSelection> {
    let (key, _, dictionary) = resolve_named_dictionary(document, value)?;
    Some(match key {
        Some(key) => FontSelection {
            id: FontId::Indirect(key),
            direct: None,
        },
        None => FontSelection {
            id: FontId::Direct(content, offset),
            direct: Some(Arc::new(dictionary.clone())),
        },
    })
}

#[derive(Debug)]
enum Encoding {
    WinAnsi,
    CffSimple(cff::Encoding),
    CffCid,
    Type3(BTreeMap<u8, PdfName>),
    Identity(CidToGid),
    Unsupported,
}

#[derive(Debug)]
enum CidToGid {
    Identity,
    Map(Vec<u8>),
}

impl CidToGid {
    fn gid(&self, cid: u16) -> Option<u16> {
        match self {
            Self::Identity => Some(cid),
            Self::Map(bytes) => {
                let offset = usize::from(cid).checked_mul(2)?;
                let end = offset.checked_add(2)?;
                Some(u16::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
            }
        }
    }
}

fn encoding(
    document: &ParsedDocument,
    font: &Dictionary,
    metric_font: &Dictionary,
    limits: &ResourceLimits,
) -> Result<Encoding> {
    let subtype = semantic_names::name(document, metric_font.get("Subtype"));
    let embedded_subtype = super::font_program_dictionary(document, font)
        .and_then(|program| semantic_names::name(document, program.get("Subtype")));
    if subtype.is_some_and(|subtype| subtype.matches("Type1"))
        && embedded_subtype.is_some_and(|subtype| subtype.matches("Type1C"))
    {
        return Ok(
            cff::Encoding::read(document, font).map_or(Encoding::Unsupported, Encoding::CffSimple)
        );
    }
    if subtype.is_some_and(|subtype| subtype.matches("CIDFontType0"))
        && embedded_subtype.is_some_and(|subtype| subtype.matches("CIDFontType0C"))
        && identity_h(document, font)
    {
        return Ok(Encoding::CffCid);
    }
    if subtype.is_some_and(|subtype| subtype.matches("Type3")) {
        let differences = simple_font_differences(document, font);
        return Ok(differences.map_or(Encoding::Unsupported, Encoding::Type3));
    }
    if subtype.is_some_and(|subtype| subtype.matches("TrueType"))
        && font_descriptor_dictionary(document, font)
            .is_some_and(|descriptor| sfnt::is_win_ansi(document, font, descriptor))
    {
        return Ok(Encoding::WinAnsi);
    }
    if !subtype.is_some_and(|subtype| subtype.matches("CIDFontType2"))
        || !identity_h(document, font)
    {
        return Ok(Encoding::Unsupported);
    }
    Ok(
        match semantic_names::resolve(document, metric_font.get("CIDToGIDMap")) {
            None if metric_font.get("CIDToGIDMap").is_none() => {
                Encoding::Identity(CidToGid::Identity)
            }
            Some(CosObject::Name(name)) if name.matches("Identity") => {
                Encoding::Identity(CidToGid::Identity)
            }
            Some(CosObject::Stream(stream)) => match sfnt::decode(stream, limits)? {
                Some(bytes) if bytes.len() % 2 == 0 => Encoding::Identity(CidToGid::Map(bytes)),
                _ => Encoding::Unsupported,
            },
            _ => Encoding::Unsupported,
        },
    )
}

fn identity_h(document: &ParsedDocument, font: &Dictionary) -> bool {
    semantic_names::name(document, font.get("Encoding"))
        .is_some_and(|encoding| encoding.matches("Identity-H"))
}

fn simple_font_differences(
    document: &ParsedDocument,
    font: &Dictionary,
) -> Option<BTreeMap<u8, PdfName>> {
    let encoding = super::resolve_dictionary_value(document, font.get("Encoding"))?;
    let CosObject::Array(entries) = semantic_names::resolve(document, encoding.get("Differences"))?
    else {
        return None;
    };
    let mut mapping = BTreeMap::new();
    let mut code = None;
    for entry in entries {
        match semantic_names::resolve(document, Some(entry))? {
            CosObject::Integer(value) => code = Some(u8::try_from(*value).ok()?),
            CosObject::Name(name) => {
                let current = code?;
                mapping.insert(current, name.clone());
                code = current.checked_add(1);
            }
            _ => return None,
        }
    }
    Some(mapping)
}

fn type3_width(
    document: &ParsedDocument,
    font: &Dictionary,
    glyph: Option<&PdfName>,
    audit: &mut CidAudit,
    limits: &ResourceLimits,
) -> Result<Option<f64>> {
    let Some(glyph) = glyph else { return Ok(None) };
    let Some(procedures) = super::resolve_dictionary_value(document, font.get("CharProcs")) else {
        return Ok(None);
    };
    let procedure = procedures
        .iter()
        .find(|(name, _)| *name == glyph)
        .map(|(_, value)| value);
    let Some(stream) = stream_from_value(document, procedure) else {
        return Ok(None);
    };
    if audit.type3_procedures >= limits.max_type3_charproc_streams {
        return Err(ValidationError::LimitExceeded {
            limit: "max_type3_charproc_streams",
        }
        .into());
    }
    if limits.max_type3_charproc_ops == 0 {
        return Err(ValidationError::LimitExceeded {
            limit: "max_type3_charproc_ops",
        }
        .into());
    }
    audit.type3_procedures = audit.type3_procedures.saturating_add(1);
    let Some(bytes) = sfnt::decode(stream, limits)? else {
        return Ok(None);
    };
    match crate::content::type3_declared_width(&bytes, limits) {
        Ok(width) => Ok(width),
        Err(error @ crate::ParseError::LimitExceeded { .. }) => Err(error.into()),
        Err(_) => Ok(None),
    }
}

#[derive(Debug)]
struct FontMetrics<'a, 'program> {
    document: &'a ParsedDocument,
    dictionary: &'a Dictionary,
    encoding: &'a Encoding,
    sfnt: Option<sfnt::Metrics<'program>>,
    cff: Option<cff::Metrics<'program>>,
    cid_widths: Option<CidWidths>,
}

impl<'a, 'program> FontMetrics<'a, 'program> {
    fn read(
        document: &'a ParsedDocument,
        dictionary: &'a Dictionary,
        metric_font: &Dictionary,
        encoding: &'a Encoding,
        bytes: Option<&'program [u8]>,
    ) -> Self {
        let (sfnt, cff) = match encoding {
            Encoding::CffSimple(_) | Encoding::CffCid => (
                None,
                bytes.and_then(|bytes| {
                    cff::Metrics::read(bytes, matches!(encoding, Encoding::CffCid))
                }),
            ),
            Encoding::WinAnsi | Encoding::Identity(_) => {
                (bytes.and_then(sfnt::Metrics::read), None)
            }
            Encoding::Type3(_) | Encoding::Unsupported => (None, None),
        };
        let cid_widths = matches!(encoding, Encoding::Identity(_) | Encoding::CffCid)
            .then(|| cid_widths(document, metric_font))
            .flatten();
        Self {
            document,
            dictionary,
            encoding,
            sfnt,
            cff,
            cid_widths,
        }
    }

    fn contains_gid(&self, gid: Option<u16>) -> Option<bool> {
        if self.cff.is_some() {
            Some(gid.is_some())
        } else {
            self.sfnt
                .as_ref()
                .map(|metrics| gid.is_some_and(|gid| metrics.has_glyph(gid)))
        }
    }

    fn glyph(&self, code: u16) -> Option<u16> {
        match self.encoding {
            Encoding::WinAnsi => self
                .sfnt
                .as_ref()?
                .unicode_gid(sfnt::win_ansi_unicode(u8::try_from(code).ok()?)?),
            Encoding::Identity(map) => map.gid(code),
            Encoding::CffSimple(encoding) => self
                .cff
                .as_ref()?
                .simple_gid(encoding, u8::try_from(code).ok()?),
            Encoding::CffCid => self.cff.as_ref()?.cid_gid(code),
            Encoding::Type3(_) | Encoding::Unsupported => None,
        }
    }

    fn width(
        &self,
        code: u16,
        gid: Option<u16>,
        audit: &mut CidAudit,
        limits: &ResourceLimits,
    ) -> Result<Measurement> {
        let measured = if let Encoding::Type3(differences) = self.encoding {
            type3_width(
                self.document,
                self.dictionary,
                u8::try_from(code)
                    .ok()
                    .and_then(|code| differences.get(&code)),
                audit,
                limits,
            )?
        } else {
            gid.and_then(|gid| {
                if let Some(cff) = &self.cff {
                    cff.width(gid)
                } else {
                    self.sfnt.as_ref()?.width(gid)
                }
            })
        };
        Ok(measured.map_or(Measurement::Unavailable, Measurement::Known))
    }

    fn expected(&self, code: u16) -> Measurement {
        let expected = match self.encoding {
            Encoding::WinAnsi | Encoding::Type3(_) | Encoding::CffSimple(_) => {
                simple_width(self.document, self.dictionary, code)
            }
            Encoding::Identity(_) | Encoding::CffCid => {
                self.cid_widths.as_ref().map(|widths| widths.get(code))
            }
            Encoding::Unsupported => None,
        };
        expected.map_or(Measurement::Unavailable, Measurement::Known)
    }
}

fn font_bytes(
    embedded: Option<&crate::StreamObject>,
    supported: bool,
    limits: &ResourceLimits,
) -> Result<Option<Vec<u8>>> {
    if supported && let Some(stream) = embedded {
        sfnt::decode(stream, limits)
    } else {
        Ok(None)
    }
}

fn cid_set_bytes(graph: &ModelGraph<'_>, font: &Dictionary) -> Result<Option<Vec<u8>>> {
    let stream = font_descriptor_dictionary(graph.document, font)
        .and_then(|descriptor| stream_from_value(graph.document, descriptor.get("CIDSet")));
    font_bytes(stream, graph.materializes(CID_SET_FAMILY), graph.limits)
}

fn unavailable_program(
    document: &ParsedDocument,
    font: &Dictionary,
    sample: Sample<'_>,
) -> Measurement {
    let type3 = semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("Type3"));
    let declares_program = font_descriptor_dictionary(document, font).is_some_and(|descriptor| {
        ["FontFile", "FontFile2", "FontFile3"]
            .iter()
            .any(|name| descriptor.get(name).is_some())
    });
    if sample.text.is_some() && !type3 && !declares_program {
        Measurement::Absent
    } else {
        Measurement::Unavailable
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "font evidence shares the current content scope, rule graph, and output budget"
)]
fn inspect_font<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    id: FontId,
    font: &Dictionary,
    samples: &[Sample<'_>],
    audit: &mut CidAudit,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    let metric_font = descendant_font_dictionary(document, font).unwrap_or(font);
    let encoding = encoding(document, font, metric_font, graph.limits)?;
    let embedded = font_program_stream(document, font);
    let bytes = font_bytes(
        embedded,
        !matches!(encoding, Encoding::Unsupported),
        graph.limits,
    )?;
    let measurements = FontMetrics::read(document, font, metric_font, &encoding, bytes.as_deref());
    let cid_bits = cid_set_bytes(graph, metric_font)?;
    let mut coverage = Coverage::Verified;
    let mut seen = BTreeSet::new();
    for sample in samples {
        if let Some(text) = sample.text
            && !matches!(encoding, Encoding::Unsupported)
        {
            for string in &text.0 {
                let data = string.as_bytes();
                if matches!(encoding, Encoding::Identity(_) | Encoding::CffCid)
                    && data.len() % 2 != 0
                {
                    coverage = coverage.include(Coverage::Unavailable);
                    if graph.materializes(WIDTH_FAMILY) {
                        push_width(
                            objects,
                            document,
                            *sample,
                            Measurement::Unavailable,
                            Measurement::Unavailable,
                            content,
                            max,
                        )?;
                    }
                    continue;
                }
                for code in character_codes(data, &encoding) {
                    if !seen.insert((sample.mode, code)) {
                        continue;
                    }
                    let gid = measurements.glyph(code);
                    if matches!(encoding, Encoding::Identity(_) | Encoding::CffCid) {
                        coverage = coverage.include(check_cid(
                            code,
                            measurements.contains_gid(gid),
                            cid_bits.as_deref(),
                        ));
                    }
                    if graph.materializes(WIDTH_FAMILY) {
                        push_width(
                            objects,
                            document,
                            *sample,
                            measurements.width(code, gid, audit, graph.limits)?,
                            measurements.expected(code),
                            content,
                            max,
                        )?;
                    }
                }
            }
        } else {
            coverage = coverage.include(Coverage::Unavailable);
            if graph.materializes(WIDTH_FAMILY) && seen.insert((sample.mode, 0)) {
                push_width(
                    objects,
                    document,
                    *sample,
                    unavailable_program(document, font, *sample),
                    Measurement::Unavailable,
                    content,
                    max,
                )?;
            }
        }
    }
    if graph.materializes(CID_SET_FAMILY)
        && let Some(record) = audit.record(id, document, font, &content.context_prefix)
    {
        record.coverage = record.coverage.include(coverage);
    }
    Ok(())
}

fn character_codes<'a>(bytes: &'a [u8], encoding: &Encoding) -> impl Iterator<Item = u16> + 'a {
    let size = if matches!(encoding, Encoding::Identity(_) | Encoding::CffCid) {
        2
    } else {
        1
    };
    bytes
        .chunks_exact(size)
        .filter_map(move |chunk| match chunk {
            [single] => Some(u16::from(*single)),
            [high, low] => Some(u16::from_be_bytes([*high, *low])),
            _ => None,
        })
}

fn check_cid(cid: u16, has_glyph: Option<bool>, bits: Option<&[u8]>) -> Coverage {
    let (Some(has_glyph), Some(bits)) = (has_glyph, bits) else {
        return Coverage::Unavailable;
    };
    if cid == 0 || !has_glyph {
        return Coverage::Verified;
    }
    if bits
        .get(usize::from(cid / 8))
        .is_some_and(|byte| byte & (0x80 >> (cid % 8)) != 0)
    {
        Coverage::Verified
    } else {
        Coverage::Missing
    }
}

fn simple_width(document: &ParsedDocument, font: &Dictionary, code: u16) -> Option<f64> {
    let widths = semantic_names::resolve(document, font.get("Widths"));
    let default = || {
        font_descriptor_dictionary(document, font)
            .and_then(|descriptor| descriptor.get("MissingWidth"))
            .and_then(number)
            .unwrap_or(0.0)
    };
    match widths {
        Some(CosObject::Array(widths)) => {
            let first = u16::try_from(cos_integer(font.get("FirstChar")?)?).ok()?;
            let expected = code
                .checked_sub(first)
                .and_then(|index| widths.get(usize::from(index)));
            expected.map_or_else(|| Some(default()), number)
        }
        None => Some(default()),
        _ => None,
    }
}

#[derive(Debug)]
struct CidWidths {
    ranges: BTreeMap<u16, (u16, f64)>,
    default: f64,
}

impl CidWidths {
    fn get(&self, cid: u16) -> f64 {
        self.ranges
            .range(..=cid)
            .next_back()
            .filter(|(_, (last, _))| cid <= *last)
            .map_or(self.default, |(_, (_, width))| *width)
    }
}

fn cid_widths(document: &ParsedDocument, font: &Dictionary) -> Option<CidWidths> {
    let default = match font.get("DW") {
        Some(value) => number(semantic_names::resolve(document, Some(value))?)?,
        None => 1000.0,
    };
    let mut ranges = BTreeMap::new();
    let mut values = match font.get("W") {
        None => [].iter(),
        Some(value) => match semantic_names::resolve(document, Some(value))? {
            CosObject::Array(values) => values.iter(),
            _ => return None,
        },
    };
    while let Some(first) = values.next() {
        let first = u16::try_from(cos_integer(first)?).ok()?;
        match values.next()? {
            CosObject::Array(widths) => {
                for (index, width) in widths.iter().enumerate() {
                    let cid = first.checked_add(u16::try_from(index).ok()?)?;
                    if ranges.insert(cid, (cid, number(width)?)).is_some() {
                        return None;
                    }
                }
            }
            value => {
                let last = u16::try_from(cos_integer(value)?).ok()?;
                if last < first {
                    return None;
                }
                if ranges
                    .insert(first, (last, number(values.next()?)?))
                    .is_some()
                {
                    return None;
                }
            }
        }
    }
    let mut previous_end = None;
    for (&first, &(last, _)) in &ranges {
        if previous_end.is_some_and(|end| first <= end) {
            return None;
        }
        previous_end = Some(last);
    }
    Some(CidWidths { ranges, default })
}

fn number(value: &CosObject) -> Option<f64> {
    match value {
        CosObject::Integer(integer) => super::i64_to_f64(*integer).ok(),
        CosObject::Real(real) if real.is_finite() => Some(*real),
        _ => None,
    }
}

fn push_width<'a>(
    objects: &mut Vec<ModelObjectRef<'a>>,
    document: &'a ParsedDocument,
    sample: Sample<'_>,
    program: Measurement,
    dictionary: Measurement,
    content: &ContentStreamModel<'_>,
    max: usize,
) -> Result<()> {
    let path = sample
        .location
        .path
        .as_ref()
        .map_or(content.context_prefix.as_str(), BoundedText::as_str);
    let glyph_path = if let Some(inherited) = &content.initial_text_state {
        format!("{path}/fontState[{}]", inherited.identity())
    } else {
        path.to_owned()
    };
    let evidence = WidthEvidence {
        mode: sample.mode,
        program,
        dictionary,
    };
    push_linked(
        objects,
        model(
            document,
            WIDTH_FAMILY,
            Evidence::Width(evidence),
            objects.len(),
            &glyph_path,
        ),
        max,
    )
}

fn model<'a>(
    document: &'a ParsedDocument,
    family: &'static str,
    evidence: Evidence,
    ordinal: usize,
    context: &str,
) -> ModelObjectRef<'a> {
    let mut model = GenericModel::new_value(
        document,
        family,
        ModelValue::Null,
        ordinal,
        format!("{context}/{family}[{ordinal}]"),
    );
    model.supertypes.clear();
    model.glyph_evidence = Some(evidence);
    ModelObjectRef::Generic(model)
}
