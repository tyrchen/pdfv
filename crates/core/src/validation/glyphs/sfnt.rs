//! SFNT metrics and PDF TrueType character-code mapping.

use std::{cell::OnceCell, collections::BTreeMap, fmt};

use read_fonts::{
    FontRead, FontRef, ReadError, TableProvider,
    ps::agl,
    tables::cmap::{
        Cmap, CmapSubtable, ConstantMapGroup, EncodingRecord, PlatformId, SequentialMapGroup,
    },
    types::{GlyphId, Tag},
};

use super::{
    super::{
        cos_integer, resolve_dictionary_value, semantic_names, stream_from_value, unknown_property,
    },
    Presence,
};
use crate::{
    Dictionary, ModelValue, ParseError, ParsedDocument, PropertyName, ResourceLimits, Result,
    StreamObject,
};

pub(crate) const PROGRAM: &str = "trueTypeFontProgram";
pub(in crate::validation) const PROGRAM_PROPERTIES: &[&str] = &[
    "isSymbolic",
    "nrCmaps",
    "cmap10Present",
    "cmap30Present",
    "cmap31Present",
];

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::validation) struct CmapLayout {
    count: u16,
    mac_roman: bool,
    windows_symbol: bool,
    windows_unicode: bool,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::validation) struct ProgramEvidence {
    pub(super) symbolic: bool,
    pub(super) cmap: Option<CmapLayout>,
}

impl ProgramEvidence {
    pub(super) fn property(self, name: &PropertyName) -> Result<ModelValue> {
        if name.as_str() == "isSymbolic" {
            return Ok(ModelValue::Bool(self.symbolic));
        }
        let Some(cmap) = self.cmap else {
            return unknown_property(name);
        };
        match name.as_str() {
            "nrCmaps" => Ok(ModelValue::Number(f64::from(cmap.count))),
            "cmap10Present" => Ok(ModelValue::Bool(cmap.mac_roman)),
            "cmap30Present" => Ok(ModelValue::Bool(cmap.windows_symbol)),
            "cmap31Present" => Ok(ModelValue::Bool(cmap.windows_unicode)),
            _ => unknown_property(name),
        }
    }
}

pub(super) fn decode(stream: &StreamObject, limits: &ResourceLimits) -> Result<Option<Vec<u8>>> {
    Ok(match read_bytes(stream, limits)? {
        ProgramBytes::Decoded(bytes) => Some(bytes),
        ProgramBytes::Malformed | ProgramBytes::Unavailable => None,
    })
}

#[derive(Debug)]
pub(super) enum ProgramBytes {
    Decoded(Vec<u8>),
    Malformed,
    Unavailable,
}

impl ProgramBytes {
    pub(super) fn as_slice(&self) -> Option<&[u8]> {
        match self {
            Self::Decoded(bytes) => Some(bytes),
            _ => None,
        }
    }
}

pub(super) fn read_bytes(stream: &StreamObject, limits: &ResourceLimits) -> Result<ProgramBytes> {
    let mut limits = limits.clone();
    limits.max_stream_decode_bytes = limits
        .max_stream_decode_bytes
        .min(limits.max_embedded_font_bytes);
    match stream.fully_decoded_bytes(&limits, None) {
        Ok(bytes) => Ok(ProgramBytes::Decoded(bytes)),
        Err(error @ ParseError::LimitExceeded { .. }) => Err(error.into()),
        Err(ParseError::StreamDecode { .. }) => Ok(ProgramBytes::Malformed),
        Err(_) => Ok(ProgramBytes::Unavailable),
    }
}

pub(super) struct Metrics<'data> {
    font: FontRef<'data>,
    glyphs: u16,
    units_per_em: u16,
    builtin_widths: [OnceCell<Option<f64>>; 256],
    post_names: OnceCell<Option<BTreeMap<&'data str, u16>>>,
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
            builtin_widths: [const { OnceCell::new() }; 256],
            post_names: OnceCell::new(),
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
        let cmap = self.cmap().ok()??;
        let records = cmap.encoding_records();
        let record = records.iter().find(|record| {
            record.platform_id() == PlatformId::Windows && record.encoding_id() == 1
        })?;
        let table = bounded_subtable(&cmap, *record).ok()?;
        mapped_gid(&table, unicode).filter(|gid| *gid != 0)
    }

    pub(super) fn named_gid(&self, name: &str) -> Option<u16> {
        if let Some(unicode) = agl::name_to_chars(name).next()
            && let Some(gid) = self.unicode_gid(u32::from(unicode))
            && self.has_glyph(gid)
        {
            return Some(gid);
        }
        let cmap = self.cmap().ok()?;
        if let Some(cmap) = &cmap
            && let Some(code) = super::simple::mac_os_code(name)
            && let Some(gid) = cmap
                .encoding_records()
                .iter()
                .find(|record| {
                    record.platform_id() == PlatformId::Macintosh && record.encoding_id() == 0
                })
                .and_then(|record| bounded_subtable(cmap, *record).ok())
                .and_then(|table| mapped_gid(&table, u32::from(code)))
                .filter(|gid| *gid != 0 && self.has_glyph(*gid))
        {
            return Some(gid);
        }
        if let Some(gid) = self.post_gid(name) {
            return Some(gid);
        }
        cmap.is_some().then_some(0)
    }

    fn post_gid(&self, name: &str) -> Option<u16> {
        self.post_names
            .get_or_init(|| {
                let post = self.font.post().ok()?;
                let mut names = BTreeMap::new();
                for (gid, name) in post
                    .glyph_names()
                    .take_while(|(gid, _)| gid.to_u32() < u32::from(self.glyphs))
                {
                    if let Ok(gid) = u16::try_from(gid.to_u32()) {
                        names.entry(name).or_insert(gid);
                    }
                }
                Some(names)
            })
            .as_ref()?
            .get(name)
            .copied()
    }

    pub(super) fn named_presence(&self, name: &str) -> Presence {
        let selected = self.named_cmap_gid(name);
        if selected
            .as_ref()
            .is_ok_and(|gid| gid.is_some_and(|gid| gid != 0 && self.has_glyph(gid)))
            || self.post_gid(name).is_some_and(|gid| gid != 0)
        {
            Presence::Defined
        } else if selected.is_err()
            || (self.has_table(Tag::new(b"post")) && self.font.post().is_err())
        {
            Presence::Unavailable
        } else {
            Presence::Missing
        }
    }

    fn named_cmap_gid(&self, name: &str) -> std::result::Result<Option<u16>, ReadError> {
        let Some(cmap) = self.cmap()? else {
            return Ok(None);
        };
        let (platform, encoding, code) = if cmap_record(&cmap, PlatformId::Windows, 1).is_some() {
            (
                PlatformId::Windows,
                1,
                agl::name_to_char(name).map(u32::from),
            )
        } else {
            (
                PlatformId::Macintosh,
                0,
                super::simple::mac_os_code(name).map(u32::from),
            )
        };
        let Some((record, code)) = cmap_record(&cmap, platform, encoding).zip(code) else {
            return Ok(None);
        };
        let table = bounded_subtable(&cmap, record)?;
        Ok(mapped_gid(&table, code))
    }

    pub(super) fn builtin_width(&self, code: u8) -> Option<f64> {
        *self
            .builtin_widths
            .get(usize::from(code))?
            .get_or_init(|| self.measure_builtin_width(code))
    }

    pub(super) fn builtin_presence(&self, code: u8) -> Presence {
        let cmap = match self.cmap() {
            Ok(Some(cmap)) => cmap,
            Ok(None) => return Presence::Missing,
            Err(_) => return Presence::Unavailable,
        };
        let record = cmap_record(&cmap, PlatformId::Windows, 0)
            .or_else(|| cmap_record(&cmap, PlatformId::Macintosh, 0));
        let Some(record) = record else {
            return Presence::Missing;
        };
        let Ok(table) = bounded_subtable(&cmap, record) else {
            return Presence::Unavailable;
        };
        let mut code = u32::from(code);
        if record.platform_id() == PlatformId::Windows {
            let Some(prefix) = first_code(&table).and_then(symbol_prefix) else {
                return Presence::Missing;
            };
            code |= prefix;
        }
        mapped_gid(&table, code)
            .is_some_and(|gid| gid != 0 && self.has_glyph(gid))
            .into()
    }

    fn measure_builtin_width(&self, code: u8) -> Option<f64> {
        let Some(cmap) = self.cmap().ok()? else {
            return Some(0.0);
        };
        let gid = builtin_gid(&cmap, code)?;
        self.width(if self.has_glyph(gid) { gid } else { 0 })
    }

    pub(super) fn cmap_layout(&self) -> Option<CmapLayout> {
        let Some(cmap) = self.cmap().ok()? else {
            return Some(CmapLayout::default());
        };
        let mut layout = CmapLayout {
            count: cmap.num_tables(),
            ..CmapLayout::default()
        };
        for record in cmap.encoding_records() {
            match (record.platform_id(), record.encoding_id()) {
                (PlatformId::Macintosh, 0) => layout.mac_roman = true,
                (PlatformId::Windows, 0) => layout.windows_symbol = true,
                (PlatformId::Windows, 1) => layout.windows_unicode = true,
                _ => {}
            }
        }
        Some(layout)
    }

    fn cmap(&self) -> std::result::Result<Option<Cmap<'data>>, ReadError> {
        match self.font.cmap() {
            Ok(cmap) if cmap.encoding_records().len() != usize::from(cmap.num_tables()) => {
                Err(ReadError::InvalidArrayLen)
            }
            Ok(cmap) => Ok(Some(cmap)),
            Err(error) if self.has_table(Tag::new(b"cmap")) => Err(error),
            Err(_) => Ok(None),
        }
    }

    fn has_table(&self, tag: Tag) -> bool {
        self.font
            .table_directory()
            .table_records()
            .iter()
            .any(|record| record.tag() == tag)
    }
}

fn cmap_record(cmap: &Cmap<'_>, platform: PlatformId, encoding: u16) -> Option<EncodingRecord> {
    cmap.encoding_records()
        .iter()
        .find(|record| record.platform_id() == platform && record.encoding_id() == encoding)
        .copied()
}

fn bounded_subtable<'a>(
    cmap: &Cmap<'a>,
    record: EncodingRecord,
) -> std::result::Result<CmapSubtable<'a>, ReadError> {
    let offset =
        usize::try_from(record.subtable_offset().to_u32()).map_err(|_| ReadError::OutOfBounds)?;
    let records_end = usize::from(cmap.num_tables())
        .saturating_mul(8)
        .saturating_add(4);
    if offset < records_end {
        return Err(ReadError::OutOfBounds);
    }
    let data = cmap
        .offset_data()
        .split_off(offset)
        .ok_or(ReadError::OutOfBounds)?;
    let format = data.read_at::<u16>(0)?;
    let length = match format {
        0 | 4 | 6 => usize::from(data.read_at::<u16>(2)?),
        10 | 12 | 13 => {
            usize::try_from(data.read_at::<u32>(4)?).map_err(|_| ReadError::OutOfBounds)?
        }
        _ => return Err(ReadError::InvalidFormat(i64::from(format))),
    };
    // Reading a subtable must not consume fields belonging to the next record.
    let table = CmapSubtable::read(data.slice(..length).ok_or(ReadError::OutOfBounds)?)?;
    if !has_complete_mapping(&table) {
        return Err(ReadError::InvalidArrayLen);
    }
    Ok(table)
}

fn has_complete_mapping(table: &CmapSubtable<'_>) -> bool {
    match table {
        CmapSubtable::Format0(table) => table.glyph_id_array().len() == 256,
        CmapSubtable::Format4(table) => {
            let count = table.seg_count_x2();
            let segments = usize::from(count / 2);
            count > 0
                && count.is_multiple_of(2)
                && [
                    table.start_code().len(),
                    table.end_code().len(),
                    table.id_range_offsets().len(),
                    table.id_delta().len(),
                ]
                .into_iter()
                .all(|len| len == segments)
        }
        CmapSubtable::Format6(table) => {
            table.glyph_id_array().len() == usize::from(table.entry_count())
        }
        CmapSubtable::Format10(table) => {
            u32::try_from(table.glyph_id_array().len()) == Ok(table.num_chars())
        }
        CmapSubtable::Format12(table) => {
            u32::try_from(table.groups().len()) == Ok(table.num_groups())
        }
        CmapSubtable::Format13(table) => {
            u32::try_from(table.groups().len()) == Ok(table.num_groups())
        }
        _ => false,
    }
}

fn builtin_gid(cmap: &Cmap<'_>, code: u8) -> Option<u16> {
    let tables = || {
        cmap.encoding_records().iter().filter_map(|record| {
            let table = bounded_subtable(cmap, *record).ok()?;
            first_code(&table)?;
            Some((record.platform_id(), record.encoding_id(), table))
        })
    };
    let code = u32::from(code);
    if let Some((_, _, symbol)) =
        tables().find(|(platform, encoding, _)| *platform == PlatformId::Windows && *encoding == 0)
        && let Some(prefix) = first_code(&symbol).and_then(symbol_prefix)
        && let Some(gid) = mapped_gid(&symbol, prefix | code).filter(|gid| *gid != 0)
    {
        return Some(gid);
    }
    if let Some((_, _, mac_roman)) = tables()
        .find(|(platform, encoding, _)| *platform == PlatformId::Macintosh && *encoding == 0)
    {
        return Some(mapped_gid(&mac_roman, code).unwrap_or(0));
    }
    // A usable cmap with no entry resolves to the .notdef glyph.
    let mut tables = tables().peekable();
    tables.peek()?;
    Some(
        tables
            .find_map(|(_, _, table)| mapped_gid(&table, code))
            .unwrap_or(0),
    )
}

fn mapped_gid(table: &CmapSubtable<'_>, code: u32) -> Option<u16> {
    u16::try_from(table.map_codepoint(code)?.to_u32()).ok()
}

fn first_code(table: &CmapSubtable<'_>) -> Option<u32> {
    match table {
        CmapSubtable::Format0(_) => Some(0),
        CmapSubtable::Format4(table) => {
            table.start_code().first().map(|code| u32::from(code.get()))
        }
        CmapSubtable::Format6(table) => Some(u32::from(table.first_code())),
        CmapSubtable::Format10(table) => Some(table.start_char_code()),
        CmapSubtable::Format12(table) => table
            .groups()
            .first()
            .map(SequentialMapGroup::start_char_code),
        CmapSubtable::Format13(table) => table
            .groups()
            .first()
            .map(ConstantMapGroup::start_char_code),
        _ => None,
    }
}

fn symbol_prefix(first_code: u32) -> Option<u32> {
    let prefix = u32::from(u16::try_from(first_code).ok()? & 0xff00);
    matches!(prefix, 0 | 0xf000 | 0xf100 | 0xf200).then_some(prefix)
}

pub(super) fn uses_builtin_encoding(
    document: &ParsedDocument,
    font: &Dictionary,
    descriptor: &Dictionary,
) -> bool {
    is_symbolic(document, descriptor)
        || font.get("Encoding").is_none_or(|value| {
            matches!(
                semantic_names::resolve(document, Some(value)),
                Some(crate::CosObject::Null)
            )
        })
}

pub(super) fn is_symbolic(document: &ParsedDocument, descriptor: &Dictionary) -> bool {
    matches!(flags(document, descriptor), Some(flags) if flags & 4 != 0)
}

fn flags(document: &ParsedDocument, descriptor: &Dictionary) -> Option<i64> {
    semantic_names::resolve(document, descriptor.get("Flags")).and_then(cos_integer)
}

pub(super) fn program_stream<'a>(
    document: &'a ParsedDocument,
    descriptor: &'a Dictionary,
) -> Option<&'a StreamObject> {
    stream_from_value(document, descriptor.get("FontFile2")).or_else(|| {
        let stream = stream_from_value(document, descriptor.get("FontFile3"))?;
        semantic_names::name(document, stream.dictionary.get("Subtype"))
            .is_some_and(|name| name.matches("OpenType"))
            .then_some(stream)
    })
}

pub(super) fn is_win_ansi(
    document: &ParsedDocument,
    font: &Dictionary,
    descriptor: &Dictionary,
) -> bool {
    if !matches!(flags(document, descriptor), Some(flags) if flags & 4 == 0 && flags & 32 != 0) {
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

#[cfg(test)]
mod tests {
    use std::{error::Error, io::Cursor, num::NonZeroU32};

    use read_fonts::{FontData, FontRead, tables::cmap::Cmap};
    use rstest::rstest;

    use super::{Metrics, Presence, builtin_gid};
    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ProfileReport, ValidationFlavour,
        ValidationOptions, Validator,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    #[rstest]
    #[case(0x0000)]
    #[case(0xf000)]
    #[case(0xf100)]
    #[case(0xf200)]
    fn test_should_use_symbol_subtable_before_macintosh(#[case] high_byte: u16) -> TestResult {
        let symbol = format6(high_byte | 0x41, 2);
        let mac = format6(0x41, 1);
        let bytes = cmap(&[(1, 0, &mac), (3, 0, &symbol)])?;
        let table = Cmap::read(FontData::new(&bytes))?;
        assert_eq!(builtin_gid(&table, 0x41), Some(2));
        assert_eq!(builtin_gid(&table, 0x42), Some(0));
        Ok(())
    }

    #[test]
    fn test_should_keep_an_explicit_notdef_in_the_macintosh_subtable() -> TestResult {
        let symbol = format6(0xf041, 0);
        let mac = format6(0x41, 0);
        let unicode = format6(0x41, 1);
        let bytes = cmap(&[(0, 3, &unicode), (1, 0, &mac), (3, 0, &symbol)])?;
        let table = Cmap::read(FontData::new(&bytes))?;
        assert_eq!(builtin_gid(&table, 0x41), Some(0));
        Ok(())
    }

    #[test]
    fn test_should_distinguish_absent_and_unreadable_cmap() -> TestResult {
        let no_cmap = sfnt(None)?;
        let malformed_cmap = sfnt(Some(&[0, 0, 0, 1]))?;
        let absent = Metrics::read(&no_cmap).ok_or("test font metrics")?;
        let malformed = Metrics::read(&malformed_cmap).ok_or("test font metrics")?;
        assert_eq!(absent.builtin_width(1), Some(0.0));
        assert_eq!(malformed.builtin_width(1), None);
        assert!(matches!(absent.builtin_presence(1), Presence::Missing));
        assert!(matches!(
            malformed.builtin_presence(1),
            Presence::Unavailable
        ));
        Ok(())
    }

    #[test]
    fn test_should_keep_cmap_lookups_inside_the_declared_subtable() -> TestResult {
        let truncated = [0, 6, 0, 10, 0, 0, 0xf0, 1, 0, 1];
        let mac = format6(1, 1);
        let mapping = cmap(&[(3, 0, &truncated), (1, 0, &mac)])?;
        let font = sfnt(Some(&mapping))?;
        let metrics = Metrics::read(&font).ok_or("test font metrics")?;
        assert!(matches!(metrics.builtin_presence(1), Presence::Unavailable));
        assert_eq!(metrics.builtin_width(1), Some(620.0));
        Ok(())
    }

    #[rstest]
    #[case("/F 10 Tf", 620, (0, 0), (0, 0))]
    #[case("/F 10 Tf () Tj", 620, (0, 0), (1, 0))]
    #[case("/F 10 Tf <01> Tj", 620, (1, 0), (1, 0))]
    #[case("/F 10 Tf <01> Tj", 600, (0, 1), (1, 0))]
    #[case("/F 10 Tf 3 Tr <01> Tj", 600, (1, 0), (1, 0))]
    #[case("/F 10 Tf /P BMC <01> Tj EMC /Span BMC <01> Tj EMC <01> Tj", 620, (2, 0), (1, 0))]
    #[case("/F 10 Tf /P <<>> BDC <01> Tj EMC /P <<>> BDC <01> Tj EMC", 620, (2, 0), (1, 0))]
    fn test_should_check_used_builtin_widths_and_program_cmap(
        #[case] text: &str,
        #[case] width: u16,
        #[case] width_checks: (usize, usize),
        #[case] program_checks: (usize, usize),
    ) -> TestResult {
        let mac = format6(1, 1);
        let mapping = cmap(&[(1, 0, &mac)])?;
        let font = sfnt(Some(&mapping))?;
        let profile = validate_truetype(text, width, font, false)?;
        assert_check(&profile, "iso-14289-1-7-21-5-1", width_checks);
        assert_check(&profile, "iso-14289-1-7-21-6-4", program_checks);
        Ok(())
    }

    #[rstest]
    #[case(1, false, false, (1, 0), (0, 1))]
    #[case(0, false, false, (0, 1), (0, 1))]
    #[case(0, true, false, (0, 1), (1, 0))]
    #[case(0, false, true, (1, 0), (0, 1))]
    #[case(2, true, false, (0, 1), (1, 0))]
    fn test_should_check_glyph_presence_independently_of_width_and_unicode(
        #[case] glyph: u16,
        #[case] to_unicode: bool,
        #[case] invisible: bool,
        #[case] presence: (usize, usize),
        #[case] unicode: (usize, usize),
    ) -> TestResult {
        let win = format6(0xf001, glyph);
        let mac = format6(1, 1);
        let cmap = cmap(&[(1, 0, &mac), (3, 0, &win)])?;
        let font = sfnt(Some(&cmap))?;
        let text = if invisible {
            "/F 10 Tf 3 Tr <01> Tj"
        } else {
            "/F 10 Tf <01> Tj"
        };
        let width = if glyph == 2 { 500 } else { 620 };
        let profile = validate_truetype(text, width, font, to_unicode)?;
        assert_check(&profile, "iso-14289-1-7-21-5-1", (1, 0));
        assert_check(&profile, "iso-14289-1-7-21-4-1-2", presence);
        assert_check(&profile, "iso-14289-1-7-21-7-1", unicode);
        assert_check(&profile, "iso-14289-1-7-21-7-2", (1, 0));
        Ok(())
    }

    fn assert_check(profile: &ProfileReport, rule: &str, expected: (usize, usize)) {
        let passed = profile
            .passed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule);
        let failed = profile
            .failed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule);
        assert_eq!((passed.count(), failed.count()), expected, "{rule}");
        assert!(
            !profile
                .unsupported_rules
                .iter()
                .any(|item| item.rule_id.0.as_str() == rule)
        );
    }

    fn validate_truetype(
        text: &str,
        width: u16,
        font: Vec<u8>,
        to_unicode: bool,
    ) -> Result<ProfileReport, Box<dyn Error>> {
        let content = format!("BT {text} ET");
        let to_unicode_ref = if to_unicode { "/ToUnicode 9 0 R" } else { "" };
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 120 120] /Contents 4 0 R /Resources << /Font << /F 5 0 R >> >> \
             >>\nendobj\n4 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n5 0 \
             obj\n<< /Type /Font /Subtype /TrueType /BaseFont /Sample /FirstChar 1 /LastChar 1 \
             /Widths [{width}] /FontDescriptor 6 0 R {to_unicode_ref} >>\nendobj\n6 0 obj\n<< \
             /Type /FontDescriptor /FontName /Sample /Flags 8 0 R /FontFile2 7 0 R >>\nendobj\n8 \
             0 obj\n6\nendobj\n7 0 obj\n<< /Length {} >>\nstream\n",
            content.len(),
            font.len()
        )
        .into_bytes();
        pdf.extend(font);
        pdf.extend(b"\nendstream\nendobj\n");
        if to_unicode {
            let map = "begincmap 1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar \
                       <01> <0041> endbfchar endcmap";
            let stream = format!(
                "9 0 obj\n<< /Length {} >>\nstream\n{map}\nendstream\nendobj\n",
                map.len()
            );
            pdf.extend(stream.as_bytes());
        }
        pdf.extend(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
        let flavour = ValidationFlavour::new("pdfua", NonZeroU32::MIN, "none")?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        let report =
            Validator::new(options)?.validate_reader(Cursor::new(pdf), InputName::memory())?;
        report
            .profile_reports
            .into_iter()
            .next()
            .ok_or_else(|| "missing PDF/UA profile".into())
    }

    fn format6(first: u16, gid: u16) -> [u8; 12] {
        let [first_high, first_low] = first.to_be_bytes();
        let [gid_high, gid_low] = gid.to_be_bytes();
        [
            0, 6, 0, 12, 0, 0, first_high, first_low, 0, 1, gid_high, gid_low,
        ]
    }

    fn cmap(records: &[(u16, u16, &[u8])]) -> Result<Vec<u8>, Box<dyn Error>> {
        const HEADER_BYTES: u32 = 4;
        const RECORD_BYTES: u32 = 8;
        let count = u16::try_from(records.len())?;
        let mut offset = HEADER_BYTES + RECORD_BYTES * u32::from(count);
        let mut bytes = Vec::from([0, 0]);
        bytes.extend(count.to_be_bytes());
        for (platform, encoding, table) in records {
            bytes.extend(platform.to_be_bytes());
            bytes.extend(encoding.to_be_bytes());
            bytes.extend(offset.to_be_bytes());
            offset += u32::try_from(table.len())?;
        }
        for (_, _, table) in records {
            bytes.extend_from_slice(table);
        }
        Ok(bytes)
    }

    fn sfnt(cmap: Option<&[u8]>) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut head = vec![0; 18];
        head.extend(1000_u16.to_be_bytes());
        head.resize(54, 0);
        let mut hhea = vec![0; 34];
        hhea.extend(2_u16.to_be_bytes());
        let maxp = [0, 0, 0x50, 0, 0, 2];
        let hmtx = [0x01, 0xf4, 0, 0, 0x02, 0x6c, 0, 0];
        let mut tables: Vec<(&[u8; 4], &[u8])> = vec![
            (b"head", &head),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
            (b"maxp", &maxp),
        ];
        if let Some(table) = cmap {
            tables.insert(0, (b"cmap", table));
        }
        let count = u16::try_from(tables.len())?;
        let mut offset = 12 + 16 * u32::from(count);
        let mut bytes = Vec::from([0, 1, 0, 0]);
        bytes.extend(count.to_be_bytes());
        bytes.extend([0; 6]);
        for (name, table) in &tables {
            let length = u32::try_from(table.len())?;
            bytes.extend_from_slice(*name);
            bytes.extend([0; 4]);
            bytes.extend(offset.to_be_bytes());
            bytes.extend(length.to_be_bytes());
            offset += length;
        }
        for (_, table) in tables {
            bytes.extend_from_slice(table);
        }
        Ok(bytes)
    }
}
