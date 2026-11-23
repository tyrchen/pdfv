//! SFNT metrics and PDF TrueType character-code mapping.

use std::fmt;

use read_fonts::{
    FontRef, TableProvider,
    tables::cmap::{CmapSubtable, PlatformId},
    types::GlyphId,
};

use super::super::{cos_integer, resolve_dictionary_value, semantic_names};
use crate::{Dictionary, ParsedDocument, PdfvError, ResourceLimits, Result, StreamObject};

pub(super) fn decode(stream: &StreamObject, limits: &ResourceLimits) -> Result<Option<Vec<u8>>> {
    let mut limits = limits.clone();
    limits.max_stream_decode_bytes = limits
        .max_stream_decode_bytes
        .min(limits.max_embedded_font_bytes);
    match stream.decoded_bytes(&limits) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error @ crate::ParseError::LimitExceeded { .. }) => Err(PdfvError::Parse(error)),
        Err(_) => Ok(None),
    }
}

pub(super) struct Metrics<'data> {
    font: FontRef<'data>,
    glyphs: u16,
    units_per_em: u16,
}

impl fmt::Debug for Metrics<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SfntMetrics")
            .field("glyphs", &self.glyphs)
            .field("units_per_em", &self.units_per_em)
            .finish_non_exhaustive()
    }
}

impl<'data> Metrics<'data> {
    pub(super) fn read(bytes: &'data [u8]) -> Option<Self> {
        let font = FontRef::new(bytes).ok()?;
        let glyphs = font.maxp().ok()?.num_glyphs();
        let units_per_em = font.head().ok()?.units_per_em();
        font.hmtx().ok()?;
        (glyphs != 0 && (16..=16_384).contains(&units_per_em)).then_some(Self {
            font,
            glyphs,
            units_per_em,
        })
    }

    pub(super) fn has_glyph(&self, gid: u16) -> bool {
        gid < self.glyphs
    }

    pub(super) fn width(&self, gid: u16) -> Option<f64> {
        if !self.has_glyph(gid) {
            return None;
        }
        let advance = self
            .font
            .hmtx()
            .ok()?
            .advance(GlyphId::new(u32::from(gid)))?;
        Some(f64::from(advance) * 1000.0 / f64::from(self.units_per_em))
    }

    pub(super) fn unicode_gid(&self, unicode: u32) -> Option<u16> {
        let cmap = self.font.cmap().ok()?;
        let records = cmap.encoding_records();
        let record = records.iter().find(|record| {
            record.platform_id() == PlatformId::Windows && record.encoding_id() == 1
        })?;
        let table = record.subtable(cmap.offset_data()).ok()?;
        if !matches!(
            table,
            CmapSubtable::Format0(_)
                | CmapSubtable::Format4(_)
                | CmapSubtable::Format6(_)
                | CmapSubtable::Format10(_)
                | CmapSubtable::Format12(_)
                | CmapSubtable::Format13(_)
        ) {
            return None;
        }
        let gid = u16::try_from(table.map_codepoint(unicode)?.to_u32()).ok()?;
        (gid != 0).then_some(gid)
    }
}

pub(super) fn is_win_ansi(
    document: &ParsedDocument,
    font: &Dictionary,
    descriptor: &Dictionary,
) -> bool {
    if !matches!(descriptor.get("Flags").and_then(cos_integer), Some(flags) if flags & 4 == 0 && flags & 32 != 0)
    {
        return false;
    }
    let encoding = font.get("Encoding");
    semantic_names::name(document, encoding).is_some_and(|name| name.matches("WinAnsiEncoding"))
        || resolve_dictionary_value(document, encoding).is_some_and(|dictionary| {
            dictionary.get("Differences").is_none()
                && semantic_names::name(document, dictionary.get("BaseEncoding"))
                    .is_some_and(|name| name.matches("WinAnsiEncoding"))
        })
}

pub(super) fn win_ansi_unicode(code: u8) -> Option<u32> {
    const EXTENDED: [u16; 32] = [
        0x20ac, 0x2022, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x2022, 0x017d, 0x2022, 0x2022, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x2022, 0x017e, 0x0178,
    ];
    match code {
        0x20..=0x7e | 0xa1..=0xac | 0xae..=0xff => Some(u32::from(code)),
        0x7f => Some(0x2022),
        0x80..=0x9f => EXTENDED
            .get(usize::from(code.saturating_sub(0x80)))
            .copied()
            .map(u32::from),
        0xa0 => Some(0x20),
        0xad => Some(0x2d),
        _ => None,
    }
}
