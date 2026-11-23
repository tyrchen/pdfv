//! Unicode destinations for declarative character maps.

use std::sync::OnceLock;

use super::{Code, Interval, Lexer, Ranges, Token, decode, parse_map, whitespace};
use crate::{ResourceLimits, Result, StreamObject};

const MAX_RECORDS: usize = 32_768;
const MAX_DESTINATION_BYTES: usize = 256;

#[derive(Debug)]
pub(in crate::validation) struct UnicodeMap {
    mappings: UnicodeMappings,
    inherits: bool,
}

#[derive(Debug)]
pub(in crate::validation) enum Lookup {
    Value(String),
    Missing,
    Invalid,
    Unavailable,
}

impl UnicodeMap {
    pub(in crate::validation) fn lookup(&self, code: Code) -> Lookup {
        let Some(index) = self.mappings.ranges.get(code.len, code.value()) else {
            return if self.inherits {
                Lookup::Unavailable
            } else {
                Lookup::Missing
            };
        };
        self.mappings
            .destinations
            .get(usize::try_from(index).unwrap_or(usize::MAX))
            .and_then(|destination| destination.decode(code.value()))
            .map_or(Lookup::Invalid, Lookup::Value)
    }
}

pub(in crate::validation) fn load_unicode(
    stream: &StreamObject,
    limits: &ResourceLimits,
) -> Result<Option<UnicodeMap>> {
    Ok(decode(stream, limits)?
        .and_then(|bytes| parse_unicode(&bytes, stream.dictionary.get("UseCMap").is_some())))
}

fn parse_unicode(bytes: &[u8], inherits: bool) -> Option<UnicodeMap> {
    let map = parse_map(bytes, inherits, true)?;
    if !map.unicode || (!map.inherits && map.spaces.is_empty()) {
        return None;
    }
    Some(UnicodeMap {
        mappings: map
            .unicode_mappings
            .map_or_else(UnicodeMappings::default, |data| *data),
        inherits: map.inherits,
    })
}

macro_rules! collection_map {
    ($file:literal) => {{
        static MAP: OnceLock<Option<UnicodeMap>> = OnceLock::new();
        MAP.get_or_init(|| {
            parse_unicode(
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/cmap/", $file)),
                false,
            )
        })
        .as_ref()
    }};
}

pub(in crate::validation) fn registry_map(ordering: &str) -> Option<&'static UnicodeMap> {
    match ordering {
        "Japan1" => collection_map!("Adobe-Japan1-UCS2"),
        "GB1" => collection_map!("Adobe-GB1-UCS2"),
        "CNS1" => collection_map!("Adobe-CNS1-UCS2"),
        "Korea1" => collection_map!("Adobe-Korea1-UCS2"),
        "KR" => collection_map!("Adobe-KR-UCS2"),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub(super) struct UnicodeMappings {
    ranges: Ranges,
    destinations: Vec<Destination>,
}

#[derive(Debug)]
struct Destination {
    bytes: Box<[u8]>,
    range_start: Option<u32>,
}

impl Destination {
    fn decode(&self, code: u32) -> Option<String> {
        let mut bytes = self.bytes.to_vec();
        if let Some(start) = self.range_start {
            increment(&mut bytes, code.checked_sub(start)?)?;
        }
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_be_bytes);
        char::decode_utf16(units)
            .collect::<std::result::Result<String, _>>()
            .ok()
    }
}

impl UnicodeMappings {
    pub(super) fn read_block(
        &mut self,
        lexer: &mut Lexer<'_>,
        count: u32,
        is_range: bool,
    ) -> Option<()> {
        for _ in 0..count {
            let low = lexer.next_code()?;
            let high = if is_range { lexer.next_code()? } else { low };
            // Lookup takes an already delimited font character code. Some
            // producers declare a two-byte ToUnicode codespace for one-byte
            // font encodings, while still supplying one-byte mapping keys.
            if low.len != high.len || low.value() > high.value() {
                return None;
            }
            self.read_destination(lexer, low, high, is_range)?;
        }
        let end = if is_range {
            b"endbfrange".as_slice()
        } else {
            b"endbfchar".as_slice()
        };
        matches!(lexer.token()?, Token::Word(word) if word == end).then_some(())
    }

    fn read_destination(
        &mut self,
        lexer: &mut Lexer<'_>,
        low: Code,
        high: Code,
        is_range: bool,
    ) -> Option<()> {
        let start = low.value();
        let end = high.value();
        let first_cid = u32::try_from(self.destinations.len()).ok()?;
        let sequential = match lexer.token()? {
            Token::Hex(text) => {
                let bytes = destination_bytes(text)?;
                let mut last = bytes.clone();
                increment(&mut last, end.checked_sub(start)?)?;
                self.push(bytes, is_range.then_some(start))?;
                false
            }
            Token::Word(b"[") if is_range => {
                let size = usize::try_from(end.checked_sub(start)?.checked_add(1)?).ok()?;
                if self.destinations.len().checked_add(size)? > MAX_RECORDS {
                    return None;
                }
                for _ in 0..size {
                    let Token::Hex(text) = lexer.token()? else {
                        return None;
                    };
                    self.push(destination_bytes(text)?, None)?;
                }
                if !matches!(lexer.token()?, Token::Word(b"]")) {
                    return None;
                }
                true
            }
            _ => return None,
        };
        self.ranges.insert(
            low.len,
            start,
            Interval {
                end,
                first_cid,
                sequential,
            },
        )
    }

    fn push(&mut self, bytes: Box<[u8]>, range_start: Option<u32>) -> Option<()> {
        if self.destinations.len() >= MAX_RECORDS {
            return None;
        }
        self.destinations.push(Destination { bytes, range_start });
        Some(())
    }
}

fn destination_bytes(input: &[u8]) -> Option<Box<[u8]>> {
    let mut digits = input.iter().copied().filter(|byte| !whitespace(*byte));
    let mut bytes = Vec::new();
    while let Some(high) = digits.next() {
        if bytes.len() >= MAX_DESTINATION_BYTES {
            return None;
        }
        bytes.push(
            (super::hex_digit(high)? << 4) | digits.next().map_or(Some(0), super::hex_digit)?,
        );
    }
    (!bytes.is_empty() && bytes.len().is_multiple_of(2)).then(|| bytes.into_boxed_slice())
}

fn increment(bytes: &mut [u8], mut amount: u32) -> Option<()> {
    for byte in bytes.iter_mut().rev() {
        if amount == 0 {
            return Some(());
        }
        let (value, overflow) = byte.overflowing_add(u8::try_from(amount & 255).ok()?);
        *byte = value;
        amount = (amount >> 8).checked_add(u32::from(overflow))?;
    }
    (amount == 0).then_some(())
}

#[cfg(test)]
mod tests {
    use super::{Lookup, parse_unicode, registry_map};
    use crate::{Result, ValidationError, validation::cmap::Code};

    #[test]
    fn test_should_load_all_packaged_character_collections() -> Result<()> {
        let first = Code::from_slice(&[0, 1]).ok_or_else(test_error)?;
        for ordering in ["Japan1", "GB1", "CNS1", "Korea1", "KR"] {
            let map = registry_map(ordering).ok_or(ValidationError::SubsystemUnavailable {
                subsystem: ordering,
            })?;
            assert!(matches!(map.lookup(first), Lookup::Value(_)), "{ordering}");
        }
        let letter = Code::from_slice(&[0, 0x29]).ok_or_else(test_error)?;
        let map = registry_map("Japan1").ok_or_else(test_error)?;
        assert!(matches!(map.lookup(letter), Lookup::Value(text) if text == "H"));
        Ok(())
    }

    #[test]
    fn test_should_decode_length_sensitive_overlays_and_utf16_destinations() -> Result<()> {
        let data = br"
begincmap
2 begincodespacerange <10> <FF> <0000> <0FFF> endcodespacerange
2 beginbfrange
<21> <24> <0041>
<0021> <0023> [<D83DDE00> <0041FEFF0042> <0000>]
endbfrange
1 beginbfchar <22> <00610062> endbfchar
endcmap";
        let map = parse_unicode(data, false).ok_or_else(test_error)?;
        for (source, expected) in [
            (&[0x21][..], "A"),
            (&[0x22][..], "ab"),
            (&[0x23][..], "C"),
            (&[0, 0x21][..], "😀"),
            (&[0, 0x22][..], "A\u{FEFF}B"),
            (&[0, 0x23][..], "\0"),
        ] {
            let code = Code::from_slice(source).ok_or_else(test_error)?;
            assert!(
                matches!(map.lookup(code), Lookup::Value(text) if text == expected),
                "{source:?}"
            );
        }
        Ok(())
    }

    fn test_error() -> ValidationError {
        ValidationError::SubsystemUnavailable {
            subsystem: "testUnicodeCMap",
        }
    }
}
