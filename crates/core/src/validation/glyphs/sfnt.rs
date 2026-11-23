//! SFNT metrics and PDF TrueType character-code mapping.

use std::{cell::OnceCell, fmt};

use read_fonts::{
    FontRef, ReadError, TableProvider,
    tables::cmap::{Cmap, CmapSubtable, ConstantMapGroup, PlatformId, SequentialMapGroup},
    types::{GlyphId, Tag},
};

use super::super::{
    cos_integer, resolve_dictionary_value, semantic_names, stream_from_value, unknown_property,
};
use crate::{
    Dictionary, ModelValue, ParsedDocument, PdfvError, PropertyName, ResourceLimits, Result,
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
    builtin_widths: [OnceCell<Option<f64>>; 256],
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
        mapped_gid(&table, unicode).filter(|gid| *gid != 0)
    }

    pub(super) fn builtin_width(&self, code: u8) -> Option<f64> {
        *self
            .builtin_widths
            .get(usize::from(code))?
            .get_or_init(|| self.measure_builtin_width(code))
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
            Ok(cmap) => Ok(Some(cmap)),
            Err(error)
                if self
                    .font
                    .table_directory()
                    .table_records()
                    .iter()
                    .any(|record| record.tag() == Tag::new(b"cmap")) =>
            {
                Err(error)
            }
            Err(_) => Ok(None),
        }
    }
}

fn builtin_gid(cmap: &Cmap<'_>, code: u8) -> Option<u16> {
    let tables = || {
        cmap.encoding_records().iter().filter_map(|record| {
            let table = record.subtable(cmap.offset_data()).ok()?;
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

    use super::{Metrics, builtin_gid};
    use crate::{
        FlavourSelection, InputName, MaxDisplayedFailures, ValidationFlavour, ValidationOptions,
        Validator,
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
        let content = format!("BT {text} ET");
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type \
             /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R \
             /MediaBox [0 0 120 120] /Contents 4 0 R /Resources << /Font << /F 5 0 R >> >> \
             >>\nendobj\n4 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n5 0 \
             obj\n<< /Type /Font /Subtype /TrueType /BaseFont /Sample /FirstChar 1 /LastChar 1 \
             /Widths [{width}] /FontDescriptor 6 0 R >>\nendobj\n6 0 obj\n<< /Type \
             /FontDescriptor /FontName /Sample /Flags 8 0 R /FontFile2 7 0 R >>\nendobj\n8 0 \
             obj\n6\nendobj\n7 0 obj\n<< /Length {} >>\nstream\n",
            content.len(),
            font.len()
        )
        .into_bytes();
        pdf.extend(font);
        pdf.extend(b"\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        let flavour = ValidationFlavour::new("pdfua", NonZeroU32::MIN, "none")?;
        let options = ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .record_passed_assertions(true)
            .max_failed_assertions_per_rule(MaxDisplayedFailures::new(NonZeroU32::MAX))
            .build();
        let report =
            Validator::new(options)?.validate_reader(Cursor::new(pdf), InputName::memory())?;
        let profile = report
            .profile_reports
            .first()
            .ok_or("missing PDF/UA profile")?;
        for (rule, expected) in [
            ("iso-14289-1-7-21-5-1", width_checks),
            ("iso-14289-1-7-21-6-4", program_checks),
        ] {
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
        Ok(())
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
