//! Decode PDF text strings used by validation rules.

use std::char::REPLACEMENT_CHARACTER;

use crate::{CosObject, Dictionary, ObjectKey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PuaSource {
    Direct(bool),
    Reference(ObjectKey),
}

impl PuaSource {
    pub(crate) fn from_object(value: &CosObject) -> Option<Self> {
        match value {
            CosObject::String(text) => Some(Self::Direct(contains_pua(text.as_bytes()))),
            CosObject::Reference(key) => Some(Self::Reference(*key)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReplacementText {
    pub(crate) actual_text: Option<PuaSource>,
    pub(crate) alt: Option<PuaSource>,
}

impl ReplacementText {
    pub(crate) fn from_dictionary(dictionary: &Dictionary) -> Self {
        Self {
            actual_text: dictionary
                .get("ActualText")
                .and_then(PuaSource::from_object),
            alt: dictionary.get("Alt").and_then(PuaSource::from_object),
        }
    }
}

pub(crate) fn is_pua(value: char) -> bool {
    matches!(value, '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}')
}

pub(crate) fn contains_pua(bytes: &[u8]) -> bool {
    if let Some(utf8) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return utf8
            .utf8_chunks()
            .any(|chunk| chunk.valid().chars().any(is_pua));
    }
    if let Some(utf16) = bytes.strip_prefix(b"\xfe\xff") {
        let (units, _) = utf16.as_chunks::<2>();
        return char::decode_utf16(units.iter().copied().map(u16::from_be_bytes))
            .filter_map(Result::ok)
            .any(is_pua);
    }
    // PDFDocEncoding has no private-use assignments.
    false
}

pub(crate) fn decode(bytes: &[u8]) -> String {
    if let Some(utf8) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    if let Some(utf16) = bytes.strip_prefix(b"\xfe\xff") {
        let (units, remainder) = utf16.as_chunks::<2>();
        let decoded = char::decode_utf16(units.iter().copied().map(u16::from_be_bytes));
        let mut text: String = decoded
            .map(|value| value.unwrap_or(REPLACEMENT_CHARACTER))
            .collect();
        if !remainder.is_empty() {
            text.push(REPLACEMENT_CHARACTER);
        }
        return text;
    }
    bytes.iter().copied().map(pdf_doc_encoding).collect()
}

fn pdf_doc_encoding(byte: u8) -> char {
    // PDFDocEncoding is ISO-8859-1 with these reassigned positions.
    const DIACRITICS: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    const SYMBOLS: [char; 31] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚',
        '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž',
    ];
    match byte {
        0x18..=0x1f => DIACRITICS
            .get(usize::from(byte.saturating_sub(0x18)))
            .copied()
            .unwrap_or(REPLACEMENT_CHARACTER),
        0x80..=0x9e => SYMBOLS
            .get(usize::from(byte.saturating_sub(0x80)))
            .copied()
            .unwrap_or(REPLACEMENT_CHARACTER),
        0x7f | 0x9f => REPLACEMENT_CHARACTER,
        0xa0 => '€',
        _ => char::from(byte),
    }
}
