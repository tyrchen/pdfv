//! Unicode evidence from character codes used by text-show operators.

use std::{
    collections::{BTreeSet, HashSet},
    str,
    sync::OnceLock,
};

use read_fonts::ps::agl;

use super::{
    ContentStreamModel, Evidence, ModelGraph, ModelObjectRef, Sample, SimpleProgram, cff, model,
    simple, type1,
};
use crate::{
    CosObject, Dictionary, ParsedDocument, Result, pdf_text,
    validation::{
        cmap::{self, CMap, Code, UnicodeLookup, UnicodeMap},
        descendant_font_dictionary, push_linked, resolve_dictionary_value, semantic_names,
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
struct SimpleMap<'a> {
    encoding: Option<simple::Encoding>,
    cff: Option<cff::Metrics<'a>>,
    type1: Option<&'a type1::Program>,
    require_listed_name: bool,
}

impl<'a> SimpleMap<'a> {
    fn read(
        document: &ParsedDocument,
        font: &Dictionary,
        program: &'a SimpleProgram,
        require_listed_name: bool,
    ) -> Self {
        Self {
            encoding: simple::Encoding::read(document, font),
            cff: program.cff(),
            type1: program.type1(),
            require_listed_name,
        }
    }

    fn lookup(&self, code: u32) -> UnicodeLookup {
        let Ok(code) = u8::try_from(code) else {
            return UnicodeLookup::Unavailable;
        };
        let Some(encoding) = &self.encoding else {
            return UnicodeLookup::Unavailable;
        };
        if let Some(name) = encoding.name(code) {
            return str::from_utf8(name).map_or(UnicodeLookup::Invalid, |name| {
                glyph_name(name, self.require_listed_name)
            });
        }
        if let Some(program) = self.type1 {
            return str::from_utf8(program.embedded_name(code))
                .map_or(UnicodeLookup::Invalid, |name| {
                    glyph_name(name, self.require_listed_name)
                });
        }
        self.cff.as_ref().map_or(UnicodeLookup::Unavailable, |cff| {
            cff.embedded_name(code)
                .map_or(UnicodeLookup::Missing, |name| {
                    glyph_name(name, self.require_listed_name)
                })
        })
    }
}

fn glyph_name(name: &str, require_listed_name: bool) -> UnicodeLookup {
    if require_listed_name && !listed_glyph_name(name) {
        return UnicodeLookup::Missing;
    }
    let text: String = agl::name_to_chars(name).collect();
    if text.is_empty() {
        UnicodeLookup::Missing
    } else {
        UnicodeLookup::Value(text)
    }
}

fn listed_glyph_name(name: &str) -> bool {
    static LEGACY_NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    // The Adobe Glyph List covers the names in the PDF Symbol font.
    LEGACY_NAMES
        .get_or_init(|| {
            include_str!("../../../data/fonts/adobe-glyph-list.txt")
                .lines()
                .filter(|line| !line.starts_with('#'))
                .filter_map(|line| line.split_once(';').map(|(name, _)| name))
                .collect()
        })
        .contains(name)
}

#[derive(Debug)]
struct GlyphMap<'a> {
    source: SourceEncoding,
    direct: DirectMap,
    collection: Option<&'static UnicodeMap>,
    simple: Option<SimpleMap<'a>>,
}

impl<'a> GlyphMap<'a> {
    fn read(graph: &ModelGraph<'_>, font: &Dictionary, program: &'a SimpleProgram) -> Result<Self> {
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
            let pdfa1 = graph.active_flavour.as_ref().is_some_and(|flavour| {
                flavour.family.as_str() == "pdfa" && flavour.part.get() == 1
            });
            let only_cmap = pdfa1 && subtype.is_some_and(|name| name.matches("Type3"));
            (
                SourceEncoding::SingleByte,
                None,
                (!only_cmap).then(|| {
                    SimpleMap::read(
                        document,
                        font,
                        program,
                        pdfa1 && super::is_type1(document, font),
                    )
                }),
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

#[allow(
    clippy::too_many_arguments,
    reason = "glyph checks reuse both their parsed font program and the marked-content use sites"
)]
pub(super) fn inspect<'a>(
    content: &ContentStreamModel<'_>,
    graph: &ModelGraph<'a>,
    font: &Dictionary,
    program: &SimpleProgram,
    samples: &[Sample<'_>],
    replacement_offsets: &BTreeSet<u64>,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    let document = graph.document;
    let map = GlyphMap::read(graph, font, program)?;
    let check_unicode = graph.materializes(FAMILY)
        && content.object_type.as_str() != crate::validation::language::CONTENT_FAMILY;
    let keep_marked_context = graph
        .active_flavour
        .as_ref()
        .is_some_and(|flavour| flavour.family.as_str() == "pdfa" && flavour.part.get() == 1);
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
                let mark = if keep_marked_context {
                    sample.marked_content
                } else {
                    None
                };
                let unicode = check_unicode && seen.insert(Some((code, sample.mode, mark)));
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::glyph_name;
    use crate::validation::cmap::UnicodeLookup;

    #[rstest]
    #[case("Aacute", true)]
    #[case("integral", true)]
    #[case("uni0041", false)]
    #[case("a1", false)]
    #[case("A.alt", false)]
    fn test_should_require_a_listed_type1_name_for_pdfa1_fallback(
        #[case] name: &str,
        #[case] listed: bool,
    ) {
        assert_eq!(
            matches!(glyph_name(name, true), UnicodeLookup::Value(_)),
            listed
        );
    }
}
