//! Mapping and advance measurements for raw CFF font programs embedded in PDFs.

use std::{collections::BTreeMap, fmt};

use read_fonts::{
    model::glyph::outline::NullSink,
    ps::{Sid, cff::CffFontRef},
    types::GlyphId,
};

use super::simple::Encoding;

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
