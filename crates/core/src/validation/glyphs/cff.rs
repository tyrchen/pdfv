//! Mapping and advance measurements for raw CFF font programs embedded in PDFs.

use std::{collections::BTreeMap, fmt};

use read_fonts::{
    model::glyph::outline::NullSink,
    ps::{PredefinedEncoding, Sid, agl, cff::CffFontRef},
    types::GlyphId,
};

use super::{sfnt, simple_font_differences};
use crate::{Dictionary, ParsedDocument, PdfName};

#[derive(Debug)]
pub(super) enum SimpleEncoding {
    Embedded,
    Standard,
    WinAnsi,
}

#[derive(Debug)]
pub(super) struct Encoding {
    base: SimpleEncoding,
    differences: BTreeMap<u8, PdfName>,
}

impl Encoding {
    pub(super) fn read(document: &ParsedDocument, font: &Dictionary) -> Option<Self> {
        let value = font.get("Encoding");
        let dictionary = super::super::resolve_dictionary_value(document, value);
        let base_value = dictionary.map_or(value, |dictionary| dictionary.get("BaseEncoding"));
        let base = match super::super::semantic_names::name(document, base_value) {
            Some(name) if name.matches("StandardEncoding") => SimpleEncoding::Standard,
            Some(name) if name.matches("WinAnsiEncoding") => SimpleEncoding::WinAnsi,
            None if base_value.is_none() => SimpleEncoding::Embedded,
            _ => return None,
        };
        let differences =
            if dictionary.is_some_and(|dictionary| dictionary.get("Differences").is_some()) {
                simple_font_differences(document, font)?
            } else {
                BTreeMap::new()
            };
        Some(Self { base, differences })
    }
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
        if let Some(name) = encoding.differences.get(&code) {
            return self.names.get(name.as_bytes()).copied();
        }
        let mut name = [0; 32];
        let base = match encoding.base {
            SimpleEncoding::Embedded => {
                return u16::try_from(self.font.encoding()?.map(code)?.to_u32()).ok();
            }
            SimpleEncoding::Standard => PredefinedEncoding::Standard.name(code),
            SimpleEncoding::WinAnsi => agl::char_to_name(sfnt::win_ansi_unicode(code)?, &mut name)?,
        };
        self.names.get(base.as_bytes()).copied()
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
