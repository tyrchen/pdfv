//! Font-use and glyph evidence from text-show operators and their graphics state.

mod cff;
mod cid_set;
mod sfnt;
mod simple;
mod type1;
mod unicode;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    ptr,
    sync::Arc,
};

pub(crate) use sfnt::PROGRAM as TRUE_TYPE_PROGRAM;
pub(super) use sfnt::PROGRAM_PROPERTIES as TRUE_TYPE_PROGRAM_PROPERTIES;
use sfnt::ProgramBytes;

use super::{
    ContentStreamModel, ContentStreamSummary, EffectiveResources, GenericModel, GlyphContent,
    ModelGraph, ModelObjectRef, OperatorFact, ResourceContent, ResourceFamily, array_values,
    cmap::{self, CMap, CidSelection, Code},
    cos_integer, descendant_font_value, font_descriptor_dictionary, font_name, font_program_stream,
    optional_string_model_value, page_dictionary, push_linked, resolve_dictionary_value,
    resolve_named_dictionary, resource_content_stream, resource_value, semantic_names,
    stream_from_value, strip_subset_font_prefix, tagged_content, unknown_property,
};
use crate::{
    BoundedText, CosObject, Dictionary, ModelValue, ObjectKey, ObjectLocation, ObjectTypeName,
    ParsedDocument, PdfName, PdfString, PropertyName, ResourceLimits, Result, StreamObject,
    ValidationError,
    content::{MarkedContentSpan, TextOperand},
};

pub(crate) const WIDTH_FAMILY: &str = "glyphWidth";
pub(crate) const USED_FONT: &str = "usedFont";
pub(crate) const USED_CID_FONT: &str = "usedCidFont";
pub(super) const USED_FONT_PROPERTIES: &[&str] = &["Subtype", "renderingMode", "containsFontFile"];
pub(super) const USED_CID_FONT_PROPERTIES: &[&str] = &["Subtype", "renderingMode", "CIDToGIDMap"];
pub(crate) const CID_SET_FAMILY: &str = "usedCidSet";
pub(crate) const CHAR_SET_FAMILY: &str = "usedType1CharSet";
pub(crate) const UNICODE_FAMILY: &str = unicode::FAMILY;
pub(crate) const PUA_FAMILY: &str = unicode::PUA_FAMILY;

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

#[derive(Clone, Copy, Debug)]
enum Presence {
    Defined,
    Missing,
    NoProgram,
    Unavailable,
}

impl From<bool> for Presence {
    fn from(defined: bool) -> Self {
        if defined {
            Self::Defined
        } else {
            Self::Missing
        }
    }
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
    FontUse(FontUse),
    Width(WidthEvidence),
    CidSet(CidSet),
    CharSet(CharSet),
    UnresolvedCidFont,
    Unicode(unicode::Mapping),
    ActualText(Option<bool>),
    TrueTypeProgram(sfnt::ProgramEvidence),
}

#[derive(Clone, Debug)]
pub(super) struct FontUse {
    subtype: Option<String>,
    mode: Option<u8>,
    embedded: Option<bool>,
    cid_map: Option<&'static str>,
}

impl FontUse {
    fn accepts_mode(&self, mode: Option<u8>) -> bool {
        match self.mode {
            Some(3) => mode != Some(3),
            None => mode.is_some_and(|mode| mode != 3),
            Some(_) => false,
        }
    }

    fn is_cid(&self) -> bool {
        matches!(
            self.subtype.as_deref(),
            Some("CIDFontType0" | "CIDFontType2")
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WidthEvidence {
    mode: Option<u8>,
    presence: Presence,
    program: Measurement,
    dictionary: Measurement,
}

impl WidthEvidence {
    fn without_selection(sample: Sample<'_>, presence: Presence) -> Self {
        let program = if sample.text.is_some() && matches!(presence, Presence::NoProgram) {
            Measurement::Absent
        } else {
            Measurement::Unavailable
        };
        Self {
            mode: sample.mode,
            presence,
            program,
            dictionary: Measurement::Unavailable,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct CidSet {
    name: Option<String>,
    embedded: bool,
    present: bool,
    coverage: Coverage,
}

#[derive(Clone, Debug)]
pub(super) struct CharSet {
    name: Option<String>,
    declaration: Option<String>,
    embedded: bool,
    coverage: Coverage,
}

impl Evidence {
    pub(super) fn property(&self, name: &PropertyName) -> Result<ModelValue> {
        match (self, name.as_str()) {
            (Self::FontUse(evidence), "Subtype") => {
                Ok(optional_string_model_value(evidence.subtype.clone()))
            }
            (Self::FontUse(evidence), "containsFontFile") => evidence.embedded.map_or_else(
                || unknown_property(name),
                |embedded| Ok(ModelValue::Bool(embedded)),
            ),
            (Self::FontUse(evidence), "CIDToGIDMap") => Ok(optional_string_model_value(
                evidence.cid_map.map(str::to_owned),
            )),
            (
                Self::FontUse(FontUse {
                    mode: None,
                    embedded: Some(true),
                    ..
                }),
                "renderingMode",
            ) => Ok(ModelValue::Null),
            (
                Self::FontUse(FontUse { mode, .. }) | Self::Width(WidthEvidence { mode, .. }),
                "renderingMode",
            ) => mode.map_or_else(
                || unknown_property(name),
                |mode| Ok(ModelValue::Number(f64::from(mode))),
            ),
            (Self::Width(evidence), "widthFromFontProgram") => measurement(evidence.program, name),
            (Self::Width(evidence), "isGlyphPresent") => match evidence.presence {
                Presence::Defined => Ok(ModelValue::Bool(true)),
                Presence::Missing => Ok(ModelValue::Bool(false)),
                Presence::NoProgram => Ok(ModelValue::Null),
                Presence::Unavailable => unknown_property(name),
            },
            (Self::Width(evidence), "widthFromDictionary") => {
                measurement(evidence.dictionary, name)
            }
            (Self::CidSet(evidence), "fontName") => {
                Ok(super::optional_string_model_value(evidence.name.clone()))
            }
            (Self::CidSet(evidence), "containsFontFile") => Ok(ModelValue::Bool(evidence.embedded)),
            (Self::CidSet(evidence), "containsCIDSet") => Ok(ModelValue::Bool(evidence.present)),
            (Self::CidSet(evidence), "cidSetListsAllGlyphs") => match evidence.coverage {
                Coverage::Verified => Ok(ModelValue::Bool(true)),
                Coverage::Missing => Ok(ModelValue::Bool(false)),
                Coverage::Unavailable => unknown_property(name),
            },
            (Self::CharSet(evidence), "fontName") => {
                Ok(super::optional_string_model_value(evidence.name.clone()))
            }
            (Self::CharSet(evidence), "CharSet") => Ok(super::optional_string_model_value(
                evidence.declaration.clone(),
            )),
            (Self::CharSet(evidence), "containsFontFile") => {
                Ok(ModelValue::Bool(evidence.embedded))
            }
            (Self::CharSet(evidence), "charSetListsAllGlyphs") => match evidence.coverage {
                Coverage::Verified => Ok(ModelValue::Bool(true)),
                Coverage::Missing => Ok(ModelValue::Bool(false)),
                Coverage::Unavailable => unknown_property(name),
            },
            (Self::Unicode(unicode::Mapping::Unavailable), _) => unknown_property(name),
            (Self::Unicode(mapping), "hasUnicodeMapping") => Ok(ModelValue::Bool(matches!(
                mapping,
                unicode::Mapping::Mapped { .. }
            ))),
            (Self::Unicode(mapping), "hasForbiddenUnicode") => Ok(ModelValue::Bool(matches!(
                mapping,
                unicode::Mapping::Mapped {
                    contains_forbidden: true,
                    ..
                }
            ))),
            (Self::ActualText(Some(value)), "hasRequiredActualText") => {
                Ok(ModelValue::Bool(*value))
            }
            (Self::TrueTypeProgram(program), _) => program.property(name),
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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum ProgramFontId {
    Indirect(ObjectKey),
    Direct(usize),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum UsedFontId {
    Font(ProgramFontId),
    DirectDescendant(ProgramFontId),
}

#[derive(Clone, Debug)]
pub(super) struct FontSelection {
    id: FontId,
    program_id: ProgramFontId,
    direct: Option<Arc<Dictionary>>,
}

impl FontSelection {
    pub(super) fn object_key(&self) -> Option<ObjectKey> {
        match self.id {
            FontId::Indirect(key) => Some(key),
            FontId::Direct(..) => None,
        }
    }

    pub(super) fn dictionary_identity(&self) -> ProgramFontId {
        self.program_id
    }

    pub(super) fn identity(&self) -> String {
        format!("{:?}", self.id)
    }

    pub(super) fn dictionary<'a>(&'a self, document: &'a ParsedDocument) -> Option<&'a Dictionary> {
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
    used_fonts: BTreeMap<UsedFontId, (FontUse, String)>,
    measured_ua1_widths: BTreeSet<WidthUse>,
    cidsets: BTreeMap<UsedFontId, (CidSet, String)>,
    charsets: BTreeMap<ProgramFontId, (CharSet, String)>,
    modeled_cmaps: BTreeSet<FontId>,
    modeled_true_type: BTreeSet<ProgramFontId>,
    unresolved: Option<String>,
    type3_procedures: u64,
    used_procedures: u64,
}

impl CidAudit {
    fn record_font_use(
        &mut self,
        document: &ParsedDocument,
        id: UsedFontId,
        font: &Dictionary,
        samples: &[Sample<'_>],
        context: &str,
    ) {
        // A single visible use invalidates the embedding exception for this font,
        // even when a different stream uses the font invisibly first.
        let Some(mode) = samples
            .iter()
            .map(|sample| sample.mode)
            .max_by_key(|mode| match mode {
                Some(3) => 0,
                None => 1,
                Some(_) => 2,
            })
        else {
            return;
        };
        match self.used_fonts.entry(id) {
            Entry::Occupied(mut entry) => {
                let (record, path) = entry.get_mut();
                if record.accepts_mode(mode) {
                    record.mode = mode;
                    context.clone_into(path);
                }
            }
            Entry::Vacant(entry) => {
                let subtype = semantic_names::name(document, font.get("Subtype"))
                    .map(|name| String::from_utf8_lossy(name.as_bytes()).into_owned());
                let cid_map = match semantic_names::resolve(document, font.get("CIDToGIDMap")) {
                    Some(CosObject::Name(name)) if name.matches("Identity") => Some("Identity"),
                    Some(CosObject::Stream(_)) => Some("Custom"),
                    _ => None,
                };
                entry.insert((
                    FontUse {
                        subtype,
                        mode,
                        embedded: Some(font_program_stream(document, font).is_some()),
                        cid_map,
                    },
                    context.to_owned(),
                ));
            }
        }
    }

    fn record_font_properties(
        &mut self,
        graph: &ModelGraph<'_>,
        selection: (ProgramFontId, &Dictionary),
        descendant: Option<(UsedFontId, &Dictionary)>,
        simple_program: &SimpleProgram,
        samples: &[Sample<'_>],
        content: &ContentStreamModel<'_>,
    ) -> Result<()> {
        let (id, font) = selection;
        let path = &content.context_prefix;
        if graph.materializes(USED_FONT) || graph.materializes(USED_CID_FONT) {
            self.record_font_use(graph.document, UsedFontId::Font(id), font, samples, path);
            if let Some((id, font)) = descendant {
                self.record_font_use(graph.document, id, font, samples, path);
            }
        }
        if !content.font_use_only() && graph.materializes(CHAR_SET_FAMILY) {
            self.record_char_set(graph, id, font, simple_program, samples, path)?;
        }
        Ok(())
    }

    fn record_embedded_program(
        &mut self,
        id: UsedFontId,
        embedded: bool,
        program: &ProgramBytes,
        metrics: bool,
    ) -> bool {
        // A missing stream or confirmed program corruption is conclusive.
        let usable = match (embedded, program) {
            (false, _) | (_, ProgramBytes::Malformed) => Some(false),
            (true, ProgramBytes::Decoded(_)) => Some(metrics),
            (true, ProgramBytes::Unavailable) => None,
        };
        if let Some((record, _)) = self.used_fonts.get_mut(&id) {
            record.embedded = usable;
        }
        usable == Some(false)
    }

    fn append_cmaps<'a>(
        &mut self,
        graph: &ModelGraph<'a>,
        font: &FontSelection,
        dictionary: &Dictionary,
        path: &str,
        objects: &mut Vec<ModelObjectRef<'a>>,
        max: usize,
    ) -> Result<()> {
        if cmap::FAMILIES
            .iter()
            .any(|family| graph.materializes(family))
            && semantic_names::name(graph.document, dictionary.get("Subtype"))
                .is_some_and(|name| name.matches("Type0"))
            && self.modeled_cmaps.insert(font.id)
        {
            cmap::append_encoding_models(graph, dictionary.get("Encoding"), path, objects, max)?;
        }
        Ok(())
    }

    fn record_char_set(
        &mut self,
        graph: &ModelGraph<'_>,
        id: ProgramFontId,
        font: &Dictionary,
        program: &SimpleProgram,
        samples: &[Sample<'_>],
        path: &str,
    ) -> Result<()> {
        let document = graph.document;
        if !is_type1(document, font) {
            return Ok(());
        }
        let descriptor = font_descriptor_dictionary(document, font);
        let value = descriptor.and_then(|descriptor| descriptor.get("CharSet"));
        let declaration = match semantic_names::resolve(document, value) {
            Some(CosObject::String(text)) => Some(text.as_bytes()),
            _ => None,
        };
        let used_only = graph
            .active_flavour
            .as_ref()
            .is_some_and(|flavour| flavour.family.as_str() == "pdfa" && flavour.part.get() == 1);
        let coverage = if !used_only && self.charsets.contains_key(&id) {
            return Ok(());
        } else if let Some(declaration) = declaration {
            char_set_coverage(
                document,
                font,
                program,
                declaration,
                used_only.then_some(samples),
            )
        } else {
            Coverage::Missing
        };
        match self.charsets.entry(id) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let record = &mut entry.get_mut().0;
                record.coverage = record.coverage.include(coverage);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                if let Some(declaration) = declaration {
                    graph.charge_text_bytes(declaration.len())?;
                }
                entry.insert((
                    CharSet {
                        name: font_name(document, font),
                        declaration: declaration
                            .map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
                        embedded: font_program_stream(document, font).is_some(),
                        coverage,
                    },
                    path.to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn true_type_model<'a>(
        &mut self,
        id: ProgramFontId,
        document: &'a ParsedDocument,
        descriptor: Option<&Dictionary>,
        metrics: &sfnt::Metrics<'_>,
        context: &str,
        ordinal: usize,
    ) -> Option<ModelObjectRef<'a>> {
        self.modeled_true_type.insert(id).then(|| {
            let evidence = sfnt::ProgramEvidence {
                symbolic: descriptor
                    .is_some_and(|descriptor| sfnt::is_symbolic(document, descriptor)),
                cmap: metrics.cmap_layout(),
            };
            model(
                document,
                TRUE_TYPE_PROGRAM,
                Evidence::TrueTypeProgram(evidence),
                ordinal,
                context,
            )
        })
    }

    fn record_cid_set(
        &mut self,
        graph: &ModelGraph<'_>,
        id: UsedFontId,
        font: &Dictionary,
        context: &str,
        used_coverage: Option<Coverage>,
    ) -> Result<()> {
        let document = graph.document;
        if !semantic_names::name(document, font.get("Subtype"))
            .is_some_and(|name| name.matches("CIDFontType0") || name.matches("CIDFontType2"))
        {
            return Ok(());
        }
        match self.cidsets.entry(id) {
            Entry::Occupied(mut entry) => {
                if let Some(coverage) = used_coverage {
                    let evidence = &mut entry.get_mut().0;
                    evidence.coverage = evidence.coverage.include(coverage);
                }
            }
            Entry::Vacant(entry) => {
                let stream = cid_set_stream(document, font);
                let name = font_name(document, font);
                let mut embedded = font_program_stream(document, font).is_some();
                let subset = name
                    .as_deref()
                    .is_some_and(|name| strip_subset_font_prefix(name) != name);
                let coverage = if let Some(coverage) = used_coverage {
                    if stream.is_some() {
                        coverage
                    } else {
                        Coverage::Missing
                    }
                } else if let Some(stream) = stream.filter(|_| subset) {
                    match cid_set::complete(document, font, stream, graph.limits)? {
                        cid_set::Evaluation::Compared(coverage) => coverage,
                        cid_set::Evaluation::RejectedProgram => {
                            embedded = false;
                            Coverage::Missing
                        }
                    }
                } else {
                    Coverage::Missing
                };
                entry.insert((
                    CidSet {
                        name,
                        embedded,
                        present: stream.is_some(),
                        coverage,
                    },
                    context.to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn record_unmeasured_cid_set(
        &mut self,
        graph: &ModelGraph<'_>,
        selection: &FontSelection,
        font: &Dictionary,
        samples: &[Sample<'_>],
        context: &str,
    ) -> Result<()> {
        let coverage = if rendered_cids_only(graph) {
            if samples.iter().any(|sample| sample.has_glyphs()) {
                return Ok(());
            }
            Some(Coverage::Verified)
        } else {
            None
        };
        let (id, cid_font) = selected_descendant(graph.document, font, selection.program_id)
            .unwrap_or((UsedFontId::Font(selection.program_id), font));
        self.record_cid_set(graph, id, cid_font, context, coverage)
    }

    pub(super) fn models<'a>(
        &self,
        document: &'a ParsedDocument,
        max: usize,
    ) -> Result<Vec<ModelObjectRef<'a>>> {
        let mut models = Vec::new();
        for (record, context) in self.used_fonts.values() {
            let mut object = model(
                document,
                USED_FONT,
                Evidence::FontUse(record.clone()),
                models.len(),
                context,
            );
            if record.is_cid()
                && let ModelObjectRef::Generic(object) = &mut object
            {
                object
                    .supertypes
                    .push(ObjectTypeName::unchecked(USED_CID_FONT));
            }
            push_linked(&mut models, object, max)?;
        }
        for (ordinal, (record, context)) in self.cidsets.values().enumerate() {
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
        for (ordinal, (record, context)) in self.charsets.values().enumerate() {
            push_linked(
                &mut models,
                model(
                    document,
                    CHAR_SET_FAMILY,
                    Evidence::CharSet(record.clone()),
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

fn char_set_coverage(
    document: &ParsedDocument,
    font: &Dictionary,
    program: &SimpleProgram,
    declaration: &[u8],
    samples: Option<&[Sample<'_>]>,
) -> Coverage {
    let mut names = declaration.trim_ascii().split(|byte| *byte == b'/');
    if names.next().is_some_and(|first| !first.is_empty()) {
        return Coverage::Missing;
    }
    let mut declared = BTreeSet::new();
    for name in names {
        let name = name.trim_ascii();
        if name.is_empty() || name.iter().any(u8::is_ascii_whitespace) {
            return Coverage::Missing;
        }
        let Some(name) = decode_char_set_name(name) else {
            return Coverage::Missing;
        };
        if name.as_ref() != b".notdef" {
            declared.insert(name);
        }
    }
    let cff = program.cff();
    let type1 = program.type1();
    if let Some(samples) = samples {
        used_char_set(document, font, cff.as_ref(), type1, &declared, samples)
    } else {
        match (cff, type1) {
            (Some(program), _) => compare_char_set(&declared, program.glyph_names()),
            (None, Some(program)) => compare_char_set(&declared, program.glyph_names()),
            _ => Coverage::Unavailable,
        }
    }
}

fn decode_char_set_name(name: &[u8]) -> Option<Cow<'_, [u8]>> {
    if !name.contains(&b'#') {
        return Some(Cow::Borrowed(name));
    }
    let mut decoded = Vec::with_capacity(name.len());
    let mut bytes = name.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'#' {
            let digits = [bytes.next()?, bytes.next()?];
            if !digits.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            decoded.push(u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok()?);
        } else {
            decoded.push(byte);
        }
    }
    Some(Cow::Owned(decoded))
}

fn used_char_set(
    document: &ParsedDocument,
    font: &Dictionary,
    cff: Option<&cff::Metrics<'_>>,
    type1: Option<&type1::Program>,
    declared: &BTreeSet<Cow<'_, [u8]>>,
    samples: &[Sample<'_>],
) -> Coverage {
    let Some(encoding) = simple::Encoding::read(document, font) else {
        return Coverage::Unavailable;
    };
    if cff.is_none() && type1.is_none() {
        return Coverage::Unavailable;
    }
    let mut seen = BTreeSet::new();
    let mut result = Coverage::Verified;
    for sample in samples {
        let Some(text) = sample.text else {
            result = Coverage::Unavailable;
            continue;
        };
        for code in text.0.iter().flat_map(PdfString::as_bytes) {
            if !seen.insert(*code) {
                continue;
            }
            let name = if let Some(program) = cff {
                program
                    .simple_gid(&encoding, *code)
                    .and_then(|glyph| program.glyph_name(glyph))
            } else if let Some(program) = type1 {
                let name = program.glyph_name(&encoding, *code);
                program.contains(name).then_some(name)
            } else {
                None
            };
            if name.is_some_and(|name| name != b".notdef" && !declared.contains(name)) {
                return Coverage::Missing;
            }
        }
    }
    result
}

fn compare_char_set<'a>(
    declared: &BTreeSet<Cow<'_, [u8]>>,
    program: impl Iterator<Item = &'a [u8]>,
) -> Coverage {
    if declared
        .iter()
        .map(Cow::as_ref)
        .eq(program.filter(|name| *name != b".notdef"))
    {
        Coverage::Verified
    } else {
        Coverage::Missing
    }
}

#[derive(Debug)]
enum SimpleProgram {
    Cff(Vec<u8>),
    Type1(type1::Program),
    Unavailable,
}

impl SimpleProgram {
    fn read(graph: &ModelGraph<'_>, font: &Dictionary) -> Result<Self> {
        let document = graph.document;
        let Some(stream) = font_program_stream(document, font).filter(|_| is_type1(document, font))
        else {
            return Ok(Self::Unavailable);
        };
        if semantic_names::name(document, stream.dictionary.get("Subtype"))
            .is_some_and(|name| name.matches("Type1C"))
        {
            return Ok(sfnt::decode(stream, graph.limits)?.map_or(Self::Unavailable, Self::Cff));
        }
        let type1_stream = font_descriptor_dictionary(document, font)
            .and_then(|descriptor| stream_from_value(document, descriptor.get("FontFile")));
        if type1_stream.is_some_and(|candidate| ptr::eq(candidate, stream)) {
            return Ok(type1::Program::read(document, stream, graph.limits)?
                .map_or(Self::Unavailable, Self::Type1));
        }
        Ok(Self::Unavailable)
    }

    fn cff(&self) -> Option<cff::Metrics<'_>> {
        match self {
            Self::Cff(bytes) => cff::Metrics::read(bytes, false),
            _ => None,
        }
    }

    fn type1(&self) -> Option<&type1::Program> {
        match self {
            Self::Type1(program) => Some(program),
            _ => None,
        }
    }
}

fn is_type1(document: &ParsedDocument, font: &Dictionary) -> bool {
    semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("Type1") || name.matches("MMType1"))
}

#[derive(Clone, Copy, Debug)]
struct Sample<'a> {
    text: Option<&'a TextOperand>,
    mode: Option<u8>,
    marked_content: Option<GlyphMark>,
    location: &'a ObjectLocation,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WidthScope {
    real_content: bool,
    struct_parent: i64,
    struct_parents: i64,
}

impl WidthScope {
    fn read(content: &ContentStreamModel<'_>, document: &ParsedDocument) -> Self {
        let owner = if content.is_form {
            Some(&content.stream.dictionary)
        } else {
            content
                .page_key
                .and_then(|key| page_dictionary(document, key))
        };
        let entry = |name| {
            semantic_names::resolve(document, owner.and_then(|dictionary| dictionary.get(name)))
                .and_then(cos_integer)
                .unwrap_or_default()
        };
        Self {
            real_content: content.glyph_content == GlyphContent::Semantic,
            struct_parent: entry("StructParent"),
            struct_parents: entry("StructParents"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WidthUse {
    font: ProgramFontId,
    scope: WidthScope,
    mode: Option<u8>,
    marked_content: Option<GlyphMark>,
    code: Code,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum GlyphMark {
    Propertyless,
    Inline(ObjectKey, u64),
    // Only equality of borrowed document dictionaries matters within this traversal.
    Resource(*const Dictionary),
}

impl GlyphMark {
    fn new(
        span: &MarkedContentSpan,
        document: &ParsedDocument,
        resources: &EffectiveResources<'_>,
        stream: ObjectKey,
    ) -> Self {
        if let Some(offset) = span.inline_properties_offset {
            Self::Inline(stream, offset)
        } else if let Some(dictionary) = tagged_content::property_list(document, resources, span) {
            Self::Resource(ptr::from_ref(dictionary))
        } else {
            Self::Propertyless
        }
    }
}

impl Sample<'_> {
    fn has_glyphs(self) -> bool {
        self.text
            .is_none_or(|text| text.0.iter().any(|string| !string.as_bytes().is_empty()))
    }
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
    replacement_offsets: &BTreeSet<u64>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<BTreeMap<u64, TextState>> {
    let scoped_only = content.object_type.as_str() == super::language::CONTENT_FAMILY;
    if content.is_form && content.initial_text_state.is_none() && !scoped_only {
        return Ok(BTreeMap::new());
    }
    let resources = content.effective_resources(graph.document)?;
    let uses = collect_uses(
        content,
        graph.document,
        &resources,
        summary,
        graph.limits,
        if graph.materializes(TRUE_TYPE_PROGRAM)
            || graph.materializes(USED_FONT)
            || graph.materializes(USED_CID_FONT)
            || graph.materializes(CHAR_SET_FAMILY)
            || graph.materializes(CID_SET_FAMILY)
            || super::unicode_name::enabled(graph)
        {
            ShowScope::Operators
        } else {
            ShowScope::Glyphs
        },
    )?;
    let mut cid_audit = graph.cid_audit_mut()?;
    if !scoped_only && rendered_cids_only(graph) && uses.unresolved.is_some() {
        cid_audit
            .unresolved
            .get_or_insert_with(|| content.context_prefix.clone());
    }
    if !content.font_use_only()
        && let Some(sample) = uses.unresolved.filter(|sample| sample.has_glyphs())
    {
        push_unresolved_glyph(content, graph, sample, replacement_offsets, objects, max)?;
    }
    for (selection, samples) in uses.fonts.values() {
        if let Some(font) = selection.dictionary(graph.document) {
            if (!scoped_only || content.font_use_only()) && graph.materializes(CID_SET_FAMILY) {
                cid_audit.record_unmeasured_cid_set(
                    graph,
                    selection,
                    font,
                    samples,
                    &content.context_prefix,
                )?;
            }
            inspect_selected_font(
                content,
                graph,
                selection,
                font,
                samples,
                replacement_offsets,
                &mut cid_audit,
                objects,
                max,
            )?;
        } else if !content.font_use_only()
            && let Some(sample) = samples.iter().copied().find(|sample| sample.has_glyphs())
        {
            push_unresolved_glyph(content, graph, sample, replacement_offsets, objects, max)?;
        }
    }
    Ok(uses.invocations)
}

#[allow(
    clippy::too_many_arguments,
    reason = "selected font evidence shares content, glyph samples, and traversal budget"
)]
fn inspect_selected_font<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    selection: &FontSelection,
    font: &Dictionary,
    samples: &[Sample<'_>],
    replacement_offsets: &BTreeSet<u64>,
    audit: &mut CidAudit,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let scoped_only = content.object_type.as_str() == super::language::CONTENT_FAMILY;
    let used_fonts = graph.materializes(USED_FONT) || graph.materializes(USED_CID_FONT);
    if (!scoped_only || content.font_use_only()) && used_fonts {
        append_used_type3_procedures(
            content, graph, selection, font, samples, audit, objects, max,
        )?;
    }
    if content.font_use_only() {
        let simple_program = SimpleProgram::read(graph, font)?;
        inspect_font(
            content,
            graph,
            selection,
            font,
            &simple_program,
            samples,
            audit,
            objects,
            max,
        )?;
        return Ok(());
    }
    if super::unicode_name::enabled(graph) && content.page_key.is_some() {
        super::unicode_name::append_used_font(graph, selection, font, content, objects, max)?;
    }
    let has_glyphs = samples.iter().any(|sample| sample.has_glyphs());
    let check_char_set = !scoped_only && graph.materializes(CHAR_SET_FAMILY);
    let check_unicode =
        graph.materializes(PUA_FAMILY) || (!scoped_only && graph.materializes(UNICODE_FAMILY));
    let check_widths = !scoped_only && graph.materializes(WIDTH_FAMILY);
    let simple_program = if check_char_set || (has_glyphs && (check_unicode || check_widths)) {
        SimpleProgram::read(graph, font)?
    } else {
        SimpleProgram::Unavailable
    };
    if !scoped_only && has_glyphs {
        audit.append_cmaps(
            graph,
            selection,
            font,
            &content.context_prefix,
            objects,
            max,
        )?;
    }
    let program_rejected = if !scoped_only
        && (check_char_set
            || used_fonts
            || graph.materializes(TRUE_TYPE_PROGRAM)
            || (has_glyphs && (check_widths || rendered_cids_only(graph))))
    {
        inspect_font(
            content,
            graph,
            selection,
            font,
            &simple_program,
            samples,
            audit,
            objects,
            max,
        )?
    } else {
        false
    };
    if check_unicode && has_glyphs {
        unicode::inspect(
            content,
            graph,
            font,
            &simple_program,
            program_rejected,
            samples,
            replacement_offsets,
            objects,
            max,
        )?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "used character procedures share the current font, text samples, and graph budget"
)]
fn append_used_type3_procedures<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    selection: &FontSelection,
    font: &Dictionary,
    samples: &[Sample<'_>],
    audit: &mut CidAudit,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    if !semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("Type3"))
    {
        return Ok(());
    }
    let (Some(procedures), Some(encoding)) = (
        resolve_dictionary_value(document, font.get("CharProcs")),
        simple::Encoding::read(document, font),
    ) else {
        return Ok(());
    };
    // PDF 1.4 does not apply Tr to the outer Type 3 glyph. ISO 32000
    // suppresses its painting in mode 3 before executing its CharProc.
    let pdfa1 = graph
        .active_flavour
        .as_ref()
        .is_some_and(|flavour| flavour.family.as_str() == "pdfa" && flavour.part.get() == 1);
    let codes: BTreeSet<_> = samples
        .iter()
        .filter(|sample| pdfa1 || sample.mode != Some(3))
        .flat_map(|sample| {
            sample.text.into_iter().flat_map(|text| {
                text.0
                    .iter()
                    .flat_map(|string| string.as_bytes().iter().map(|code| (*code, sample.mode)))
            })
        })
        .collect();
    if codes.is_empty() {
        return Ok(());
    }
    let identity = selection.identity();
    let resources = graph
        .character_resources
        .try_borrow_mut()
        .map_err(|_| ValidationError::SubsystemUnavailable {
            subsystem: "characterResources",
        })?
        .entry(identity.clone())
        .or_insert_with(|| {
            resolve_dictionary_value(document, font.get("Resources"))
                .map(|resources| Arc::new(resources.clone()))
        })
        .clone();
    let resources = super::language::CharacterResources {
        font: identity,
        dictionary: resources,
    };
    for (code, mode) in codes {
        let name = PdfName::new(
            encoding.name(code).unwrap_or(b".notdef").to_vec(),
            graph.limits,
        )?;
        let Some((Some(key), CosObject::Stream(_))) = semantic_names::resolve_reference(
            document,
            procedures.get_name(&name),
            graph.limits.max_object_depth,
        )?
        else {
            continue;
        };
        if audit.used_procedures >= graph.limits.max_type3_charproc_streams {
            return Err(ValidationError::LimitExceeded {
                limit: "max_type3_charproc_streams",
            }
            .into());
        }
        let path = format!(
            "root/page[{}]/type3CharProc[{}]",
            content.page_ordinal,
            super::object_key_text(key)
        );
        if let Some(mut procedure) = resource_content_stream(
            graph,
            key,
            content.page_key,
            content.page_ordinal,
            path,
            ResourceContent::CharProc(resources.clone()),
        )? {
            audit.used_procedures = audit.used_procedures.saturating_add(1);
            procedure.glyph_content = GlyphContent::FontUseOnly;
            procedure.initial_text_state = Some(TextState {
                mode,
                font: Some(selection.clone()),
            });
            push_linked(objects, ModelObjectRef::ContentStream(procedure), max)?;
        }
    }
    Ok(())
}

fn push_unresolved_glyph<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    sample: Sample<'_>,
    replacement_offsets: &BTreeSet<u64>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let scoped_only = content.object_type.as_str() == super::language::CONTENT_FAMILY;
    if !scoped_only && graph.materializes(WIDTH_FAMILY) {
        push_width(
            objects,
            graph.document,
            sample,
            WidthEvidence::without_selection(sample, Presence::Unavailable),
            content,
            max,
        )?;
    }
    if !scoped_only && graph.materializes(UNICODE_FAMILY) {
        unicode::push(
            content,
            graph.document,
            sample,
            unicode::Mapping::Unavailable,
            objects,
            max,
        )?;
    }
    if graph.materializes(PUA_FAMILY) {
        let replacement = sample
            .location
            .offset
            .is_some_and(|offset| replacement_offsets.contains(&offset));
        unicode::push_private_use(
            content,
            graph.document,
            sample,
            unicode::Mapping::Unavailable,
            replacement,
            objects,
            max,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShowScope {
    Glyphs,
    Operators,
}

fn collect_uses<'a>(
    content: &ContentStreamModel<'_>,
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    summary: &'a ContentStreamSummary,
    limits: &ResourceLimits,
    show_scope: ShowScope,
) -> Result<TextUses<'a>> {
    let mut uses = TextUses::default();
    let mut selections = FontSelections::new(summary);
    let mut state = content
        .initial_text_state
        .clone()
        .unwrap_or_else(TextState::initial);
    let mut saved = Vec::new();
    let mut marked = Vec::new();
    let mut spans = summary.marked_content.iter();
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
            OperatorFact::MarkedContent { op, .. } if matches!(op.as_str(), "BMC" | "BDC") => {
                if let Some(span) = spans.next() {
                    marked.push(GlyphMark::new(span, document, resources, content.key));
                }
            }
            OperatorFact::MarkedContent { op, .. } if op.as_str() == "EMC" => {
                marked.pop();
            }
            OperatorFact::TextShow { location, .. } => {
                let text = location
                    .offset
                    .and_then(|offset| summary.text_operands.get(&offset));
                let sample = Sample {
                    text,
                    mode: state.mode,
                    marked_content: marked.last().copied(),
                    location,
                };
                if show_scope == ShowScope::Glyphs && !sample.has_glyphs() {
                    continue;
                }
                if let Some(font) = &state.font {
                    uses.fonts
                        .entry(font.id)
                        .or_insert_with(|| (font.clone(), Vec::new()))
                        .1
                        .push(sample);
                } else if sample.has_glyphs()
                    && uses
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

pub(super) fn shown_fonts(
    content: &ContentStreamModel<'_>,
    document: &ParsedDocument,
    resources: &EffectiveResources<'_>,
    summary: &ContentStreamSummary,
    limits: &ResourceLimits,
) -> Result<Vec<FontSelection>> {
    Ok(collect_uses(
        content,
        document,
        resources,
        summary,
        limits,
        ShowScope::Operators,
    )?
    .fonts
    .into_values()
    .map(|(font, _samples)| font)
    .collect())
}

pub(super) fn text_invocations(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'_>,
    summary: &ContentStreamSummary,
) -> Result<BTreeMap<u64, TextState>> {
    let resources = content.effective_resources(graph.document)?;
    Ok(collect_uses(
        content,
        graph.document,
        &resources,
        summary,
        graph.limits,
        ShowScope::Glyphs,
    )?
    .invocations)
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
            program_id: ProgramFontId::Indirect(key),
            direct: None,
        },
        None => FontSelection {
            id: FontId::Direct(content, offset),
            program_id: ProgramFontId::Direct(ptr::from_ref(dictionary).addr()),
            direct: Some(Arc::new(dictionary.clone())),
        },
    })
}

#[derive(Debug)]
enum Encoding {
    WinAnsi,
    TrueTypeNamed(simple::Encoding),
    TrueTypeBuiltin,
    CffSimple(simple::Encoding),
    Type1(simple::Encoding),
    CffCid(CidEncoding),
    Type3(simple::Encoding),
    CidTrueType(CidEncoding, CidToGid),
    Unsupported,
}

#[derive(Debug)]
enum CidEncoding {
    Identity,
    Embedded(Box<CMap>),
}

impl Encoding {
    fn cid_encoding(&self) -> Option<&CidEncoding> {
        match self {
            Self::CffCid(cmap) | Self::CidTrueType(cmap, _) => Some(cmap),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum CidToGid {
    Identity,
    Map(Vec<u8>),
}

#[derive(Debug)]
enum CidMap {
    Available(CidToGid),
    Malformed,
    Unavailable,
}

impl CidMap {
    fn read(document: &ParsedDocument, font: &Dictionary, limits: &ResourceLimits) -> Result<Self> {
        const MAX_BYTES: usize = (u16::MAX as usize + 1) * 2;
        let entry = font.get("CIDToGIDMap");
        Ok(match semantic_names::resolve(document, entry) {
            None if entry.is_none() => Self::Available(CidToGid::Identity),
            Some(CosObject::Null) => Self::Available(CidToGid::Identity),
            Some(CosObject::Name(name)) if name.matches("Identity") => {
                Self::Available(CidToGid::Identity)
            }
            Some(CosObject::Stream(stream)) => match sfnt::read_bytes(stream, limits)? {
                ProgramBytes::Decoded(bytes)
                    if bytes.len().is_multiple_of(2) && bytes.len() <= MAX_BYTES =>
                {
                    Self::Available(CidToGid::Map(bytes))
                }
                ProgramBytes::Decoded(_) | ProgramBytes::Malformed => Self::Malformed,
                ProgramBytes::Unavailable => Self::Unavailable,
            },
            None => Self::Unavailable,
            _ => Self::Malformed,
        })
    }
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
    // A PDF can retain the MMType1 subtype after embedding an instantiated Type 1C program.
    if subtype.is_some_and(|subtype| subtype.matches("Type1") || subtype.matches("MMType1"))
        && embedded_subtype.is_some_and(|subtype| subtype.matches("Type1C"))
    {
        return Ok(simple::Encoding::read(document, font)
            .map_or(Encoding::Unsupported, Encoding::CffSimple));
    }
    if subtype.is_some_and(|subtype| subtype.matches("Type1") || subtype.matches("MMType1"))
        && font_descriptor_dictionary(document, font)
            .is_some_and(|descriptor| descriptor.get("FontFile").is_some())
    {
        return Ok(
            simple::Encoding::read(document, font).map_or(Encoding::Unsupported, Encoding::Type1)
        );
    }
    if subtype.is_some_and(|subtype| subtype.matches("CIDFontType0"))
        && embedded_subtype.is_some_and(|subtype| subtype.matches("CIDFontType0C"))
    {
        return Ok(
            cid_encoding(document, font, limits)?.map_or(Encoding::Unsupported, Encoding::CffCid)
        );
    }
    if subtype.is_some_and(|subtype| subtype.matches("Type3")) {
        return Ok(
            simple::Encoding::read(document, font).map_or(Encoding::Unsupported, Encoding::Type3)
        );
    }
    if subtype.is_some_and(|subtype| subtype.matches("TrueType"))
        && let Some(descriptor) = font_descriptor_dictionary(document, font)
    {
        if sfnt::program_stream(document, descriptor).is_some()
            && sfnt::uses_builtin_encoding(document, font, descriptor)
        {
            return Ok(Encoding::TrueTypeBuiltin);
        }
        if sfnt::is_win_ansi(document, font, descriptor) {
            return Ok(Encoding::WinAnsi);
        }
        if !sfnt::is_symbolic(document, descriptor)
            && let Some(names) = simple::Encoding::read(document, font)
        {
            return Ok(Encoding::TrueTypeNamed(names));
        }
    }
    if !subtype.is_some_and(|subtype| subtype.matches("CIDFontType2")) {
        return Ok(Encoding::Unsupported);
    }
    let Some(cmap) = cid_encoding(document, font, limits)? else {
        return Ok(Encoding::Unsupported);
    };
    Ok(match CidMap::read(document, metric_font, limits)? {
        CidMap::Available(map) => Encoding::CidTrueType(cmap, map),
        CidMap::Malformed | CidMap::Unavailable => Encoding::Unsupported,
    })
}

fn cid_encoding(
    document: &ParsedDocument,
    font: &Dictionary,
    limits: &ResourceLimits,
) -> Result<Option<CidEncoding>> {
    let encoding = font.get("Encoding");
    if semantic_names::name(document, encoding)
        .is_some_and(|name| name.matches("Identity-H") || name.matches("Identity-V"))
    {
        return Ok(Some(CidEncoding::Identity));
    }
    let Some(stream) = stream_from_value(document, encoding) else {
        return Ok(None);
    };
    Ok(cmap::load(stream, limits)?
        .filter(CMap::supports_cids)
        .map(|cmap| CidEncoding::Embedded(Box::new(cmap))))
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
    glyph: Option<&[u8]>,
    audit: &mut CidAudit,
    limits: &ResourceLimits,
) -> Result<Option<f64>> {
    let Some(stream) = type3_program(document, font, glyph) else {
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

fn type3_program<'a>(
    document: &'a ParsedDocument,
    font: &'a Dictionary,
    glyph: Option<&[u8]>,
) -> Option<&'a StreamObject> {
    let procedures = resolve_dictionary_value(document, font.get("CharProcs"))?;
    let glyph = glyph?;
    let procedure = std::str::from_utf8(glyph).ok().map_or_else(
        || {
            procedures
                .iter()
                .find(|(name, _)| name.as_bytes() == glyph)
                .map(|(_, value)| value)
        },
        |name| procedures.get(name),
    );
    stream_from_value(document, procedure)
}

#[derive(Debug)]
struct FontMetrics<'a, 'program> {
    document: &'a ParsedDocument,
    dictionary: &'a Dictionary,
    encoding: &'a Encoding,
    sfnt: Option<sfnt::Metrics<'program>>,
    cff: Option<cff::Metrics<'program>>,
    type1: Option<&'program type1::Program>,
    cid_widths: Option<CidWidths>,
}

impl<'a, 'program> FontMetrics<'a, 'program> {
    fn read(
        document: &'a ParsedDocument,
        dictionary: &'a Dictionary,
        metric_font: &Dictionary,
        encoding: &'a Encoding,
        bytes: Option<&'program [u8]>,
        simple_program: &'program SimpleProgram,
    ) -> Self {
        let (sfnt, cff) = match encoding {
            Encoding::CffSimple(_) => (None, simple_program.cff()),
            Encoding::CffCid(_) => (
                None,
                bytes.and_then(|bytes| {
                    cff::Metrics::read(bytes, matches!(encoding, Encoding::CffCid(_)))
                }),
            ),
            Encoding::WinAnsi
            | Encoding::TrueTypeNamed(_)
            | Encoding::TrueTypeBuiltin
            | Encoding::CidTrueType(..)
            | Encoding::Unsupported => (bytes.and_then(sfnt::Metrics::read), None),
            Encoding::Type3(_) | Encoding::Type1(_) => (None, None),
        };
        let cid_widths = encoding
            .cid_encoding()
            .is_some()
            .then(|| cid_widths(document, metric_font))
            .flatten();
        Self {
            document,
            dictionary,
            encoding,
            sfnt,
            cff,
            type1: simple_program.type1(),
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
            Encoding::WinAnsi => {
                let code = u8::try_from(code).ok()?;
                let metrics = self.sfnt.as_ref()?;
                sfnt::win_ansi_unicode(code)
                    .and_then(|unicode| metrics.unicode_gid(unicode))
                    .filter(|gid| metrics.has_glyph(*gid))
                    .or_else(|| metrics.named_gid(simple::base_name(simple::Base::WinAnsi, code)?))
            }
            Encoding::TrueTypeNamed(encoding) => {
                let name = std::str::from_utf8(encoding.name(u8::try_from(code).ok()?)?).ok()?;
                self.sfnt.as_ref()?.named_gid(name)
            }
            Encoding::CidTrueType(_, map) => map.gid(code),
            Encoding::CffSimple(encoding) => self
                .cff
                .as_ref()?
                .simple_gid(encoding, u8::try_from(code).ok()?),
            Encoding::CffCid(_) => self.cff.as_ref()?.cid_gid(code),
            Encoding::Type3(_)
            | Encoding::Type1(_)
            | Encoding::TrueTypeBuiltin
            | Encoding::Unsupported => None,
        }
    }

    fn select(&self, code: CharacterCode) -> Option<(u16, Option<u16>, bool)> {
        match code {
            CharacterCode::Direct(code) => Some((code, self.glyph(code), true)),
            CharacterCode::Cid(selection) => {
                if let Some(preferred) = selection.preferred {
                    let cid = u16::try_from(preferred).ok()?;
                    let gid = self.glyph(cid);
                    if self.has_mapped_glyph(cid, gid)? {
                        return Some((cid, gid, true));
                    }
                }
                let cid = u16::try_from(selection.fallback).ok()?;
                let gid = self.glyph(cid);
                if cid != 0 && !self.has_mapped_glyph(cid, gid)? {
                    return Some((0, self.glyph(0), false));
                }
                Some((cid, gid, false))
            }
        }
    }

    fn has_mapped_glyph(&self, cid: u16, gid: Option<u16>) -> Option<bool> {
        // In a CIDToGIDMap, GID zero designates .notdef for an unmapped CID.
        self.contains_gid(gid)
            .map(|present| present && (cid == 0 || gid != Some(0)))
    }

    fn presence(
        &self,
        code: u16,
        gid: Option<u16>,
        exact: bool,
        unavailable: Presence,
    ) -> Presence {
        if matches!(unavailable, Presence::NoProgram) {
            return unavailable;
        }
        match self.encoding {
            Encoding::TrueTypeBuiltin => match (self.sfnt.as_ref(), u8::try_from(code)) {
                (Some(metrics), Ok(code)) => metrics.builtin_presence(code),
                _ => unavailable,
            },
            Encoding::Type3(encoding) => type3_program(
                self.document,
                self.dictionary,
                u8::try_from(code).ok().and_then(|code| encoding.name(code)),
            )
            .is_some()
            .into(),
            Encoding::Type1(encoding) => match (self.type1, u8::try_from(code)) {
                (Some(program), Ok(code)) => {
                    let name = program.glyph_name(encoding, code);
                    program.contains(name).into()
                }
                _ => unavailable,
            },
            Encoding::WinAnsi | Encoding::TrueTypeNamed(_) => {
                let Ok(code) = u8::try_from(code) else {
                    return unavailable;
                };
                let name = match self.encoding {
                    Encoding::TrueTypeNamed(encoding) => encoding.name(code),
                    _ => simple::base_name(simple::Base::WinAnsi, code).map(str::as_bytes),
                };
                match (self.sfnt.as_ref(), name) {
                    (Some(metrics), Some(name)) => std::str::from_utf8(name)
                        .map_or(Presence::Missing, |name| metrics.named_presence(name)),
                    _ => unavailable,
                }
            }
            Encoding::Unsupported => unavailable,
            _ => self.contains_gid(gid).map_or(unavailable, |present| {
                (exact && present && gid != Some(0)).into()
            }),
        }
    }

    fn width(
        &self,
        code: u16,
        gid: Option<u16>,
        audit: &mut CidAudit,
        limits: &ResourceLimits,
    ) -> Result<Measurement> {
        let measured = if let Encoding::Type3(encoding) = self.encoding {
            type3_width(
                self.document,
                self.dictionary,
                u8::try_from(code).ok().and_then(|code| encoding.name(code)),
                audit,
                limits,
            )?
        } else if let Encoding::Type1(encoding) = self.encoding {
            u8::try_from(code).ok().and_then(|code| {
                let program = self.type1?;
                program.width(program.glyph_name(encoding, code))
            })
        } else if matches!(self.encoding, Encoding::TrueTypeBuiltin) {
            u8::try_from(code)
                .ok()
                .and_then(|code| self.sfnt.as_ref()?.builtin_width(code))
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
            Encoding::WinAnsi
            | Encoding::TrueTypeNamed(_)
            | Encoding::TrueTypeBuiltin
            | Encoding::Type3(_)
            | Encoding::CffSimple(_)
            | Encoding::Type1(_) => simple_width(self.document, self.dictionary, code),
            Encoding::CidTrueType(..) | Encoding::CffCid(_) => {
                self.cid_widths.as_ref().map(|widths| widths.get(code))
            }
            Encoding::Unsupported => None,
        };
        expected.map_or(Measurement::Unavailable, Measurement::Known)
    }

    fn observe(
        &self,
        sample: Sample<'_>,
        selected: Option<(u16, Option<u16>, bool)>,
        explicit_cid_fallback: bool,
        unavailable: Presence,
        audit: &mut CidAudit,
        limits: &ResourceLimits,
    ) -> Result<WidthEvidence> {
        let mut evidence = WidthEvidence::without_selection(sample, unavailable);
        if let Some((code, gid, exact)) = selected {
            evidence.presence =
                self.presence(code, gid, exact || explicit_cid_fallback, unavailable);
            if exact || explicit_cid_fallback {
                if !matches!(unavailable, Presence::NoProgram) {
                    evidence.program = self.width(code, gid, audit, limits)?;
                }
                evidence.dictionary = self.expected(code);
            } else if matches!(evidence.presence, Presence::Missing) {
                // A substituted CID has no advance for the requested glyph.
                evidence.program = Measurement::Absent;
            }
        }
        if matches!(evidence.presence, Presence::Missing)
            && matches!(self.encoding, Encoding::Type3(_))
        {
            evidence.program = Measurement::Absent;
        }
        Ok(evidence)
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

fn cid_set_stream<'a>(
    document: &'a ParsedDocument,
    font: &'a Dictionary,
) -> Option<&'a StreamObject> {
    let value = font_descriptor_dictionary(document, font)?.get("CIDSet");
    stream_from_value(document, value)
}

fn rendered_cids_only(graph: &ModelGraph<'_>) -> bool {
    graph.materializes(CID_SET_FAMILY)
        && graph
            .active_flavour
            .as_ref()
            .is_some_and(|flavour| flavour.family.as_str() == "pdfa" && flavour.part.get() == 1)
}

fn font_declares_program(document: &ParsedDocument, font: &Dictionary) -> bool {
    font_descriptor_dictionary(document, font).is_some_and(|descriptor| {
        ["FontFile", "FontFile2", "FontFile3"]
            .iter()
            .any(|name| descriptor.get(name).is_some())
    })
}

fn unavailable_presence(document: &ParsedDocument, font: &Dictionary, rejected: bool) -> Presence {
    let inline_program = semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("Type3"));
    if rejected || (!inline_program && !font_declares_program(document, font)) {
        Presence::NoProgram
    } else {
        Presence::Unavailable
    }
}

fn selected_descendant<'a>(
    document: &'a ParsedDocument,
    font: &'a Dictionary,
    id: ProgramFontId,
) -> Option<(UsedFontId, &'a Dictionary)> {
    let subtype = semantic_names::name(document, font.get("Subtype"))?;
    if !subtype.matches("Type0") {
        return None;
    }
    let (key, _, descendant) =
        resolve_named_dictionary(document, descendant_font_value(document, font)?)?;
    let id = key.map_or(UsedFontId::DirectDescendant(id), |key| {
        UsedFontId::Font(ProgramFontId::Indirect(key))
    });
    Some((id, descendant))
}

#[allow(
    clippy::too_many_arguments,
    reason = "font evidence shares the current content scope, rule graph, and output budget"
)]
fn inspect_font<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    selection: &FontSelection,
    font: &Dictionary,
    simple_program: &SimpleProgram,
    samples: &[Sample<'_>],
    audit: &mut CidAudit,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<bool> {
    let document = graph.document;
    let id = UsedFontId::Font(selection.program_id);
    let descendant = selected_descendant(document, font, selection.program_id);
    audit.record_font_properties(
        graph,
        (selection.program_id, font),
        descendant,
        simple_program,
        samples,
        content,
    )?;
    let (metric_id, metric_font) = descendant.unwrap_or((id, font));
    let is_true_type = semantic_names::name(document, font.get("Subtype"))
        .is_some_and(|name| name.matches("TrueType"));
    let subtype = semantic_names::name(document, metric_font.get("Subtype"));
    let expects_sfnt = is_true_type || subtype.is_some_and(|name| name.matches("CIDFontType2"));
    let expects_cff = subtype.is_some_and(|name| name.matches("CIDFontType0"));
    let descriptor = font_descriptor_dictionary(document, metric_font);
    let embedded = if expects_sfnt {
        descriptor.and_then(|descriptor| sfnt::program_stream(document, descriptor))
    } else {
        font_program_stream(document, metric_font)
    };
    let program = !content.font_use_only()
        && graph.materializes(TRUE_TYPE_PROGRAM)
        && is_true_type
        && embedded.is_some();
    let check_embedding = (expects_sfnt || expects_cff) && graph.materializes(USED_FONT);
    let compare_glyphs = (graph.materializes(WIDTH_FAMILY) || rendered_cids_only(graph))
        && samples.iter().any(|sample| sample.has_glyphs());
    if !(program || compare_glyphs || check_embedding) {
        return Ok(false);
    }
    let encoding = if compare_glyphs {
        encoding(document, font, metric_font, graph.limits)?
    } else {
        Encoding::Unsupported
    };
    let read_program = program
        || check_embedding
        || !matches!(
            encoding,
            Encoding::Unsupported | Encoding::CffSimple(_) | Encoding::Type1(_)
        );
    let bytes = match embedded.filter(|_| read_program) {
        Some(stream) => sfnt::read_bytes(stream, graph.limits)?,
        None => ProgramBytes::Unavailable,
    };
    let measurements = FontMetrics::read(
        document,
        font,
        metric_font,
        &encoding,
        bytes.as_slice(),
        simple_program,
    );
    let rejected_program = check_embedding
        && audit.record_embedded_program(
            metric_id,
            embedded.is_some(),
            &bytes,
            if expects_cff {
                embedded
                    .zip(bytes.as_slice())
                    .is_some_and(|(stream, bytes)| {
                        cff::embedded_cid_font(document, stream, bytes).is_some()
                    })
            } else {
                measurements.sfnt.is_some()
            },
        );
    if program
        && let Some(metrics) = &measurements.sfnt
        && let Some(model) = audit.true_type_model(
            selection.program_id,
            document,
            descriptor,
            metrics,
            &content.context_prefix,
            objects.len(),
        )
    {
        push_linked(objects, model, max)?;
    }
    if compare_glyphs {
        measure_used_glyphs(
            content,
            graph,
            selection,
            rejected_program,
            &measurements,
            samples,
            audit,
            objects,
            max,
        )?;
    }
    Ok(rejected_program)
}

#[allow(
    clippy::too_many_arguments,
    reason = "glyph evidence uses the current font metrics, rule graph, audit, and output budget"
)]
fn measure_used_glyphs<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    selection: &FontSelection,
    rejected_program: bool,
    measurements: &FontMetrics<'_, '_>,
    samples: &[Sample<'_>],
    audit: &mut CidAudit,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    let font = measurements.dictionary;
    let encoding = measurements.encoding;
    let (metric_id, metric_font) = selected_descendant(document, font, selection.program_id)
        .unwrap_or((UsedFontId::Font(selection.program_id), font));
    let check_cids = rendered_cids_only(graph);
    let cid_bits = font_bytes(
        cid_set_stream(document, metric_font),
        check_cids,
        graph.limits,
    )?;
    let mut coverage = Coverage::Verified;
    let mut seen_widths = BTreeSet::new();
    let mut seen_cids = BTreeSet::new();
    let ua1_scope = graph
        .active_flavour
        .as_ref()
        .is_some_and(|flavour| flavour.family.as_str() == "pdfua" && flavour.part.get() == 1)
        .then(|| WidthScope::read(content, document));
    let check_widths = graph.materializes(WIDTH_FAMILY);
    let unavailable = unavailable_presence(document, font, rejected_program);
    for sample in samples.iter().filter(|sample| sample.has_glyphs()) {
        let text = sample
            .text
            .filter(|_| !matches!(encoding, Encoding::Unsupported));
        let Some(text) = text else {
            coverage = coverage.include(Coverage::Unavailable);
            if check_widths && seen_widths.insert((sample.mode, sample.marked_content, 0, false)) {
                push_width(
                    objects,
                    document,
                    *sample,
                    WidthEvidence::without_selection(*sample, unavailable),
                    content,
                    max,
                )?;
            }
            continue;
        };
        for decoded in text
            .0
            .iter()
            .flat_map(|string| character_codes(string.as_bytes(), encoding.cid_encoding()))
        {
            let selection_result = decoded.and_then(|(_, code)| measurements.select(code));
            let (code, gid, exact) = selection_result.unwrap_or((0, None, false));
            if selection_result.is_none() {
                coverage = coverage.include(Coverage::Unavailable);
            } else if check_cids
                && encoding.cid_encoding().is_some()
                && seen_cids.insert((code, exact))
            {
                coverage = coverage.include(check_cid(
                    code,
                    measurements.contains_gid(gid),
                    cid_bits.as_deref(),
                ));
            }
            if !check_widths {
                continue;
            }
            let first_observation = if let (Some(scope), Some((code, _))) = (ua1_scope, decoded) {
                audit.measured_ua1_widths.insert(WidthUse {
                    font: selection.program_id,
                    scope,
                    mode: sample.mode,
                    marked_content: sample.marked_content,
                    code,
                })
            } else {
                seen_widths.insert((sample.mode, sample.marked_content, code, exact))
            };
            if first_observation {
                let evidence = measurements.observe(
                    *sample,
                    selection_result,
                    code != 0 && decoded.is_some_and(|(_, character)| character.has_cid_fallback()),
                    unavailable,
                    audit,
                    graph.limits,
                )?;
                push_width(objects, document, *sample, evidence, content, max)?;
            }
        }
    }
    if check_cids {
        audit.record_cid_set(
            graph,
            metric_id,
            metric_font,
            &content.context_prefix,
            Some(coverage),
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum CharacterCode {
    Direct(u16),
    Cid(CidSelection),
}

impl CharacterCode {
    fn has_cid_fallback(self) -> bool {
        matches!(
            self,
            Self::Cid(CidSelection {
                preferred: None,
                fallback: 1..
            })
        )
    }
}

fn character_codes<'a>(
    mut bytes: &'a [u8],
    encoding: Option<&'a CidEncoding>,
) -> impl Iterator<Item = Option<(Code, CharacterCode)>> + 'a {
    std::iter::from_fn(move || {
        if bytes.is_empty() {
            return None;
        }
        let code = match encoding {
            Some(CidEncoding::Embedded(cmap)) => cmap
                .next_character_code(&mut bytes)
                .map(|(code, cid)| (code, CharacterCode::Cid(cid))),
            Some(CidEncoding::Identity) => bytes.split_at_checked(2).and_then(|(code, rest)| {
                bytes = rest;
                let raw = Code::from_slice(code)?;
                Some((
                    raw,
                    CharacterCode::Cid(CidSelection {
                        preferred: Some(u32::from(u16::from_be_bytes(code.try_into().ok()?))),
                        fallback: 0,
                    }),
                ))
            }),
            None => bytes.split_first().and_then(|(code, rest)| {
                bytes = rest;
                Some((
                    Code::from_slice(&[*code])?,
                    CharacterCode::Direct(u16::from(*code)),
                ))
            }),
        };
        if code.is_none() {
            bytes = &[];
        }
        Some(code)
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
            .and_then(|descriptor| {
                semantic_names::resolve(document, descriptor.get("MissingWidth"))
            })
            .and_then(number)
            .unwrap_or(0.0)
    };
    match widths {
        Some(CosObject::Array(widths)) => {
            let char_bound = |name| {
                u8::try_from(cos_integer(semantic_names::resolve(
                    document,
                    font.get(name),
                )?)?)
                .ok()
                .map(u16::from)
            };
            let (Some(first), Some(last)) = (char_bound("FirstChar"), char_bound("LastChar"))
            else {
                return Some(default());
            };
            let expected = code
                .checked_sub(first)
                .filter(|_| code <= last)
                .and_then(|index| widths.get(usize::from(index)));
            expected.map_or_else(
                || Some(default()),
                |width| semantic_names::resolve(document, Some(width)).and_then(number),
            )
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
    evidence: WidthEvidence,
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
