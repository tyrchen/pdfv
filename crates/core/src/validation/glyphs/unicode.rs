//! Unicode evidence from character codes used by text-show operators.

use std::{collections::BTreeSet, str};

use read_fonts::ps::{PredefinedEncoding, agl};

use super::{
    ContentStreamModel, Evidence, ModelGraph, ModelObjectRef, Sample, cff, font_bytes, model,
    simple_font_differences,
};
use crate::{
    CosObject, Dictionary, ParsedDocument, Result, pdf_text,
    validation::{
        cmap::{self, CMap, Code, UnicodeLookup, UnicodeMap},
        descendant_font_dictionary, font_program_dictionary, font_program_stream, push_linked,
        resolve_dictionary_value, semantic_names,
    },
};

pub(in crate::validation) const FAMILY: &str = "glyphUnicode";
pub(in crate::validation) const PUA_FAMILY: &str = "glyphPrivateUse";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::validation) enum Mapping {
    Mapped {
        contains_forbidden: bool,
        contains_private_use: bool,
    },
    Unmapped,
    Unavailable,
}

impl From<UnicodeLookup> for Mapping {
    fn from(lookup: UnicodeLookup) -> Self {
        match lookup {
            UnicodeLookup::Value(text) => Self::Mapped {
                contains_forbidden: text
                    .chars()
                    .any(|ch| matches!(ch, '\0' | '\u{FFFE}' | '\u{FEFF}')),
                contains_private_use: text.chars().any(pdf_text::is_pua),
            },
            UnicodeLookup::Invalid | UnicodeLookup::Missing => Self::Unmapped,
            UnicodeLookup::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Debug)]
enum DirectMap {
    Missing,
    Identity,
    Embedded(Box<UnicodeMap>),
    Unavailable,
}

impl DirectMap {
    fn read(graph: &ModelGraph<'_>, font: &Dictionary) -> Result<Self> {
        Ok(
            match semantic_names::resolve(graph.document, font.get("ToUnicode")) {
                Some(CosObject::Stream(stream)) => cmap::load_unicode(stream, graph.limits)?
                    .map_or(Self::Unavailable, |map| Self::Embedded(Box::new(map))),
                Some(CosObject::Name(name)) if identity_name(name) => Self::Identity,
                None if font.get("ToUnicode").is_none() => Self::Missing,
                Some(CosObject::Null) => Self::Missing,
                _ => Self::Unavailable,
            },
        )
    }

    fn lookup(&self, code: Code) -> UnicodeLookup {
        match self {
            Self::Missing => UnicodeLookup::Missing,
            Self::Identity => char::from_u32(code.value()).map_or(UnicodeLookup::Invalid, |ch| {
                UnicodeLookup::Value(ch.to_string())
            }),
            Self::Embedded(map) => map.lookup(code),
            Self::Unavailable => UnicodeLookup::Unavailable,
        }
    }
}

#[derive(Debug)]
enum SourceEncoding {
    SingleByte,
    Identity,
    Embedded(Box<CMap>),
    Unavailable,
}

impl SourceEncoding {
    fn next(&self, bytes: &mut &[u8]) -> Option<(Code, Option<u32>)> {
        let width = match self {
            Self::Unavailable => return None,
            Self::SingleByte => 1,
            Self::Identity => 2,
            Self::Embedded(map) => {
                return map
                    .next_character_code(bytes)
                    .map(|(code, cid)| (code, Some(cid.preferred.unwrap_or(cid.fallback))));
            }
        };
        let (bytes_for_code, rest) = bytes.split_at_checked(width)?;
        *bytes = rest;
        let code = Code::from_slice(bytes_for_code)?;
        Some((code, (width == 2).then(|| code.value())))
    }
}

#[derive(Debug)]
enum SimpleBase {
    Standard,
    WinAnsi,
    Unspecified,
    Unavailable,
}

#[derive(Debug)]
struct SimpleMap<'a> {
    base: SimpleBase,
    differences: Option<std::collections::BTreeMap<u8, crate::PdfName>>,
    cff: Option<cff::Metrics<'a>>,
}

impl<'a> SimpleMap<'a> {
    fn read(document: &ParsedDocument, font: &Dictionary, font_bytes: Option<&'a [u8]>) -> Self {
        let value = font.get("Encoding");
        let dictionary = resolve_dictionary_value(document, value);
        let name_value = dictionary.map_or(value, |dict| dict.get("BaseEncoding"));
        let base = match semantic_names::name(document, name_value) {
            Some(name) if name.matches("StandardEncoding") => SimpleBase::Standard,
            Some(name) if name.matches("WinAnsiEncoding") => SimpleBase::WinAnsi,
            None if name_value.is_none() => SimpleBase::Unspecified,
            _ => SimpleBase::Unavailable,
        };
        let differences = if dictionary.is_some_and(|dict| dict.get("Differences").is_some()) {
            simple_font_differences(document, font)
        } else {
            Some(std::collections::BTreeMap::new())
        };
        Self {
            base,
            differences,
            cff: font_bytes.and_then(|bytes| cff::Metrics::read(bytes, false)),
        }
    }

    fn lookup(&self, code: u32) -> UnicodeLookup {
        let Ok(code) = u8::try_from(code) else {
            return UnicodeLookup::Unavailable;
        };
        let Some(differences) = &self.differences else {
            return UnicodeLookup::Unavailable;
        };
        if let Some(name) = differences.get(&code) {
            return str::from_utf8(name.as_bytes()).map_or(UnicodeLookup::Invalid, glyph_name);
        }
        match self.base {
            SimpleBase::Standard => glyph_name(PredefinedEncoding::Standard.name(code)),
            SimpleBase::WinAnsi => super::sfnt::win_ansi_unicode(code)
                .and_then(char::from_u32)
                .map_or(UnicodeLookup::Missing, |ch| {
                    UnicodeLookup::Value(ch.to_string())
                }),
            SimpleBase::Unspecified => {
                self.cff.as_ref().map_or(UnicodeLookup::Unavailable, |cff| {
                    cff.embedded_name(code)
                        .map_or(UnicodeLookup::Missing, glyph_name)
                })
            }
            SimpleBase::Unavailable => UnicodeLookup::Unavailable,
        }
    }
}

fn glyph_name(name: &str) -> UnicodeLookup {
    let text: String = agl::name_to_chars(name).collect();
    if text.is_empty() {
        UnicodeLookup::Missing
    } else {
        UnicodeLookup::Value(text)
    }
}

#[derive(Debug)]
struct GlyphMap<'a> {
    source: SourceEncoding,
    direct: DirectMap,
    collection: Option<&'static UnicodeMap>,
    simple: Option<SimpleMap<'a>>,
}

impl<'a> GlyphMap<'a> {
    fn read(
        graph: &ModelGraph<'_>,
        font: &Dictionary,
        font_bytes: Option<&'a [u8]>,
    ) -> Result<Self> {
        let document = graph.document;
        let direct = DirectMap::read(graph, font)?;
        let subtype = semantic_names::name(document, font.get("Subtype"));
        let (source, collection, simple) = if subtype.is_some_and(|name| name.matches("Type0")) {
            let encoding = semantic_names::resolve(document, font.get("Encoding"));
            let (source, collection) = match encoding {
                Some(CosObject::Name(name)) if identity_name(name) => (
                    SourceEncoding::Identity,
                    descendant_font_dictionary(document, font)
                        .and_then(|dict| collection_map(document, dict)),
                ),
                Some(CosObject::Stream(stream)) => (
                    cmap::load(stream, graph.limits)?
                        .filter(CMap::supports_cids)
                        .map_or(SourceEncoding::Unavailable, |map| {
                            SourceEncoding::Embedded(Box::new(map))
                        }),
                    collection_map(document, &stream.dictionary),
                ),
                _ => (SourceEncoding::Unavailable, None),
            };
            (source, collection, None)
        } else if subtype.is_some_and(|name| {
            ["Type1", "MMType1", "TrueType", "Type3"]
                .iter()
                .any(|subtype| name.matches(subtype))
        }) {
            (
                SourceEncoding::SingleByte,
                None,
                Some(SimpleMap::read(document, font, font_bytes)),
            )
        } else {
            (SourceEncoding::Unavailable, None, None)
        };
        Ok(Self {
            source,
            direct,
            collection,
            simple,
        })
    }

    fn lookup(&self, code: Code, cid: Option<u32>) -> Mapping {
        let mut lookup = self.direct.lookup(code);
        if matches!(lookup, UnicodeLookup::Missing) {
            lookup = self.fallback(code, cid);
        }
        lookup.into()
    }

    fn fallback(&self, code: Code, cid: Option<u32>) -> UnicodeLookup {
        if let Some(simple) = &self.simple {
            return simple.lookup(code.value());
        }
        match (self.collection, cid.and_then(|cid| u16::try_from(cid).ok())) {
            (Some(collection), Some(cid)) => Code::from_slice(&cid.to_be_bytes())
                .map_or(UnicodeLookup::Unavailable, |key| collection.lookup(key)),
            _ => UnicodeLookup::Missing,
        }
    }
}

fn identity_name(name: &crate::PdfName) -> bool {
    name.matches("Identity-H") || name.matches("Identity-V")
}

fn collection_map(document: &ParsedDocument, font: &Dictionary) -> Option<&'static UnicodeMap> {
    let system = resolve_dictionary_value(document, font.get("CIDSystemInfo"))?;
    let Some(CosObject::String(registry)) =
        semantic_names::resolve(document, system.get("Registry"))
    else {
        return None;
    };
    let Some(CosObject::String(ordering)) =
        semantic_names::resolve(document, system.get("Ordering"))
    else {
        return None;
    };
    if registry.as_bytes() != b"Adobe" {
        return None;
    }
    cmap::registry_map(str::from_utf8(ordering.as_bytes()).ok()?)
}

pub(super) fn inspect<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    font: &Dictionary,
    samples: &[Sample<'_>],
    replacement_offsets: &BTreeSet<u64>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    let is_cff = font_program_dictionary(document, font)
        .and_then(|dict| semantic_names::name(document, dict.get("Subtype")))
        .is_some_and(|name| name.matches("Type1C"));
    let program = font_bytes(font_program_stream(document, font), is_cff, graph.limits)?;
    let map = GlyphMap::read(graph, font, program.as_deref())?;
    let check_unicode = graph.materializes(FAMILY)
        && content.object_type.as_str() != crate::validation::language::CONTENT_FAMILY;
    let mut seen = BTreeSet::new();
    let mut private_use = BTreeSet::new();
    for sample in samples {
        if !sample.has_glyphs() {
            continue;
        }
        let replacement = sample
            .location
            .offset
            .is_some_and(|offset| replacement_offsets.contains(&offset));
        let decode_seen = |scope| (scope, sample.mode, sample.marked_content, replacement);
        let Some(text) = sample.text else {
            if check_unicode && seen.insert(None) {
                push(
                    content,
                    document,
                    *sample,
                    Mapping::Unavailable,
                    objects,
                    max,
                )?;
            }
            if graph.materializes(PUA_FAMILY) && private_use.insert(decode_seen(None)) {
                push_private_use(
                    content,
                    document,
                    *sample,
                    Mapping::Unavailable,
                    replacement,
                    objects,
                    max,
                )?;
            }
            continue;
        };
        for string in &text.0 {
            let mut bytes = string.as_bytes();
            while !bytes.is_empty() {
                let Some((code, cid)) = map.source.next(&mut bytes) else {
                    if check_unicode && seen.insert(None) {
                        push(
                            content,
                            document,
                            *sample,
                            Mapping::Unavailable,
                            objects,
                            max,
                        )?;
                    }
                    if graph.materializes(PUA_FAMILY) && private_use.insert(decode_seen(None)) {
                        push_private_use(
                            content,
                            document,
                            *sample,
                            Mapping::Unavailable,
                            replacement,
                            objects,
                            max,
                        )?;
                    }
                    break;
                };
                let unicode = check_unicode && seen.insert(Some((code, sample.mode)));
                let pua =
                    graph.materializes(PUA_FAMILY) && private_use.insert(decode_seen(Some(code)));
                if unicode || pua {
                    let mapping = map.lookup(code, cid);
                    if unicode {
                        push(content, document, *sample, mapping, objects, max)?;
                    }
                    if pua {
                        push_private_use(
                            content,
                            document,
                            *sample,
                            mapping,
                            replacement,
                            objects,
                            max,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn push<'a>(
    content: &ContentStreamModel<'_>,
    document: &'a ParsedDocument,
    sample: Sample<'_>,
    mapping: Mapping,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    push_evidence(
        content,
        document,
        sample,
        FAMILY,
        Evidence::Unicode(mapping),
        objects,
        max,
    )
}

pub(super) fn push_private_use<'a>(
    content: &ContentStreamModel<'_>,
    document: &'a ParsedDocument,
    sample: Sample<'_>,
    mapping: Mapping,
    replacement: bool,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let allowed = match (replacement, mapping) {
        (true, _) | (false, Mapping::Unmapped) => Some(true),
        (
            false,
            Mapping::Mapped {
                contains_private_use,
                ..
            },
        ) => Some(!contains_private_use),
        (false, Mapping::Unavailable) => None,
    };
    push_evidence(
        content,
        document,
        sample,
        PUA_FAMILY,
        Evidence::ActualText(allowed),
        objects,
        max,
    )
}

fn push_evidence<'a>(
    content: &ContentStreamModel<'_>,
    document: &'a ParsedDocument,
    sample: Sample<'_>,
    family: &'static str,
    evidence: Evidence,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let path = sample
        .location
        .path
        .as_ref()
        .map_or(content.context_prefix.as_str(), crate::BoundedText::as_str);
    let ordinal = objects.len();
    let ModelObjectRef::Generic(mut evidence) = model(document, family, evidence, ordinal, path)
    else {
        return Err(crate::ValidationError::SubsystemUnavailable {
            subsystem: "glyphUnicode",
        }
        .into());
    };
    evidence.key = sample.location.object;
    evidence.offset = sample.location.offset;
    push_linked(objects, ModelObjectRef::Generic(evidence), max)
}
