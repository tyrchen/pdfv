//! Mapping and advance measurements for raw CFF font programs embedded in PDFs.

use std::{collections::BTreeMap, fmt};

use read_fonts::{
    FontRef,
    model::glyph::outline::NullSink,
    ps::{Sid, cff::CffFontRef},
    types::{GlyphId, Tag},
};

use super::{semantic_names, simple::Encoding};
use crate::{ParsedDocument, StreamObject};

pub(super) fn embedded_cid_font<'a>(
    document: &ParsedDocument,
    program: &StreamObject,
    bytes: &'a [u8],
) -> Option<CffFontRef<'a>> {
    let open_type = semantic_names::name(document, program.dictionary.get("Subtype"))
        .is_some_and(|name| name.matches("OpenType"));
    let data = if open_type {
        FontRef::new(bytes)
            .ok()?
            .table_data(Tag::new(b"CFF "))?
            .as_bytes()
    } else {
        bytes
    };
    if data.first() != Some(&1) {
        return None;
    }
    let font = CffFontRef::new_cff(data, 0, Some(1000)).ok()?;
    (1..=u32::from(u16::MAX))
        .contains(&font.num_glyphs())
        .then_some(font)
}

pub(super) struct Metrics<'data> {
    font: CffFontRef<'data>,
    names: BTreeMap<&'data [u8], u16>,
}

impl fmt::Debug for Metrics<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CffMetrics")
            .field("glyphs", &self.font.num_glyphs())
            .finish_non_exhaustive()
    }
}

impl<'data> Metrics<'data> {
    pub(super) fn read(bytes: &'data [u8], cid: bool) -> Option<Self> {
        let font = CffFontRef::new_cff(bytes, 0, Some(1000)).ok()?;
        if font.is_cid() != cid || font.num_glyphs() > u32::from(u16::MAX) {
            return None;
        }
        let charset = font.charset()?;
        let mut names = BTreeMap::new();
        if !cid {
            for index in 0..font.num_glyphs() {
                let name = font.string(charset.string_id(GlyphId::new(index))?)?;
                if names.insert(name, u16::try_from(index).ok()?).is_some() {
                    return None;
                }
            }
        }
        Some(Self { font, names })
    }

    pub(super) fn cid_gid(&self, cid: u16) -> Option<u16> {
        u16::try_from(self.font.charset()?.glyph_id(Sid::new(cid))?.to_u32()).ok()
    }

    pub(super) fn embedded_name(&self, code: u8) -> Option<&str> {
        let glyph = self.font.encoding()?.map(code)?;
        let name = self.font.string(self.font.charset()?.string_id(glyph)?)?;
        std::str::from_utf8(name).ok()
    }

    pub(super) fn simple_gid(&self, encoding: &Encoding, code: u8) -> Option<u16> {
        if let Some(name) = encoding.name(code) {
            self.names.get(name).copied()
        } else {
            u16::try_from(self.font.encoding()?.map(code)?.to_u32()).ok()
        }
    }

    pub(super) fn glyph_names(&self) -> impl Iterator<Item = &[u8]> {
        self.names.keys().copied()
    }

    pub(super) fn glyph_name(&self, glyph: u16) -> Option<&[u8]> {
        let sid = self
            .font
            .charset()?
            .string_id(GlyphId::new(u32::from(glyph)))?;
        self.font.string(sid)
    }

    pub(super) fn width(&self, gid: u16) -> Option<f64> {
        let glyph = GlyphId::new(u32::from(gid));
        let subfont = self
            .font
            .subfont(self.font.subfont_index(glyph)?, &[])
            .ok()?;
        let width = self
            .font
            .evaluate_charstring(&subfont, glyph, &[], &mut NullSink)
            .ok()??;
        Some(f64::from(width.to_bits()) / 65_536.0)
    }
}
