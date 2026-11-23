//! Bounded parsing of declarative character maps embedded in PDF files.

use std::{collections::BTreeMap, sync::Arc};

use super::{GenericModel, ModelGraph, ModelObjectRef, push_linked, semantic_names};
use crate::{
    CosObject, Dictionary, ModelValue, ParseError, PdfName, ResourceLimits, Result, StreamObject,
};

pub(super) const FAMILIES: &[&str] = &["cMap", "cMapFile", "referencedCMap"];

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SPACE_RANGES: usize = 256;
const MAX_MAPPING_RANGES: usize = 16_384;
const MAX_BLOCK_ENTRIES: u32 = 100;

#[derive(Debug, Default)]
pub(super) struct CMap {
    spaces: Vec<Space>,
    mappings: Ranges,
    notdef: Ranges,
    writing_mode: u32,
    maximal_cid: u32,
    unicode: bool,
    inherits: bool,
}

pub(super) fn load(stream: &StreamObject, limits: &ResourceLimits) -> Result<Option<CMap>> {
    let mut limits = limits.clone();
    limits.max_stream_decode_bytes = limits.max_stream_decode_bytes.min(MAX_BYTES);
    match stream.decoded_bytes(&limits) {
        Ok(bytes) => Ok(parse(&bytes, stream.dictionary.get("UseCMap").is_some())),
        Err(error @ ParseError::LimitExceeded { .. }) => Err(error.into()),
        Err(_) => Ok(None),
    }
}

pub(super) fn append_encoding_models<'a>(
    graph: &ModelGraph<'a>,
    encoding: Option<&CosObject>,
    context: &str,
    objects: &mut Vec<ModelObjectRef<'a>>,
    max: usize,
) -> Result<()> {
    if graph.materializes("cMap")
        && let Some(model) = cmap_model(graph, encoding, "cMap", context, objects.len())
    {
        push_linked(objects, ModelObjectRef::Generic(model), max)?;
    }
    if let Some(CosObject::Stream(stream)) = semantic_names::resolve(graph.document, encoding) {
        if graph.materializes("cMapFile")
            && let Some(mut model) = cmap_model(graph, encoding, "cMapFile", context, objects.len())
        {
            model.parsed_cmap = load(stream, graph.limits)?.map(Arc::new);
            push_linked(objects, ModelObjectRef::Generic(model), max)?;
        }
        if graph.materializes("referencedCMap")
            && let Some(parent) = cmap_model(
                graph,
                stream.dictionary.get("UseCMap"),
                "referencedCMap",
                context,
                objects.len(),
            )
        {
            push_linked(objects, ModelObjectRef::Generic(parent), max)?;
        }
    }
    Ok(())
}

fn cmap_model<'a>(
    graph: &ModelGraph<'a>,
    value: Option<&CosObject>,
    family: &'static str,
    context: &str,
    ordinal: usize,
) -> Option<GenericModel<'a>> {
    let document = graph.document;
    let context = format!("{context}/{family}[{ordinal}]");
    if family != "cMap" && value.is_none() {
        return None;
    }
    let resolved = semantic_names::resolve(document, value);
    let mut named_dictionary = Dictionary::default();
    let dictionary = match resolved {
        Some(CosObject::Stream(stream)) => &stream.dictionary,
        Some(CosObject::Name(name)) => {
            named_dictionary.insert(
                PdfName::try_from(String::from("CMapName")).ok()?,
                CosObject::Name(name.clone()),
            );
            &named_dictionary
        }
        _ => &named_dictionary,
    };
    let key = match value {
        Some(CosObject::Reference(key)) => Some(*key),
        _ => None,
    };
    let offset = key.and_then(|key| document.objects.get(&key).map(|object| object.offset));
    let mut model = GenericModel::new(document, family, key, offset, dictionary, ordinal, context);
    model.direct_value = Some(ModelValue::Bool(matches!(
        resolved,
        Some(CosObject::Stream(_))
    )));
    Some(model)
}

impl CMap {
    pub(super) fn writing_mode(&self) -> u32 {
        self.writing_mode
    }

    pub(super) fn maximal_cid(&self) -> Option<u32> {
        (!self.inherits || self.maximal_cid > u32::from(u16::MAX)).then_some(self.maximal_cid)
    }

    pub(super) fn supports_horizontal_cids(&self) -> bool {
        self.writing_mode == 0 && !self.unicode && !self.inherits && !self.spaces.is_empty()
    }

    pub(super) fn next_cid(&self, input: &mut &[u8]) -> Option<CidSelection> {
        let space = self.spaces.iter().find(|space| space.matches(input))?;
        let (code, tail) = input.split_at_checked(usize::from(space.len))?;
        *input = tail;
        let code = code
            .iter()
            .fold(0_u32, |value, byte| (value << 8) | u32::from(*byte));
        Some(CidSelection {
            preferred: self.mappings.get(space.len, code),
            fallback: self.notdef.get(space.len, code).unwrap_or(0),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CidSelection {
    pub(super) preferred: Option<u32>,
    pub(super) fallback: u32,
}

#[derive(Clone, Copy, Debug)]
struct Code {
    bytes: [u8; 4],
    len: u8,
}

impl Code {
    fn value(self) -> u32 {
        self.bytes
            .iter()
            .take(usize::from(self.len))
            .fold(0_u32, |value, byte| (value << 8) | u32::from(*byte))
    }
}

#[derive(Debug)]
struct Space {
    len: u8,
    low: [u8; 4],
    high: [u8; 4],
}

impl Space {
    fn from_codes(low: Code, high: Code) -> Option<Self> {
        (low.len == high.len && low.bytes.iter().zip(high.bytes).all(|(lo, hi)| *lo <= hi))
            .then_some(Self {
                len: low.len,
                low: low.bytes,
                high: high.bytes,
            })
    }

    fn overlaps_prefix(&self, other: &Self) -> bool {
        self.low
            .iter()
            .zip(self.high)
            .zip(other.low.iter().zip(other.high))
            .take(usize::from(self.len.min(other.len)))
            .all(|((lo, hi), (other_lo, other_hi))| *lo <= other_hi && *other_lo <= hi)
    }

    fn matches(&self, input: &[u8]) -> bool {
        input.get(..usize::from(self.len)).is_some_and(|code| {
            code.iter()
                .zip(self.low.iter().zip(self.high))
                .all(|(byte, (low, high))| byte >= low && *byte <= high)
        })
    }

    // Count rectangle members no greater than the given code in byte order.
    fn count_through(&self, code: u32) -> u64 {
        let bounds = self
            .low
            .iter()
            .copied()
            .zip(self.high)
            .take(usize::from(self.len));
        let code_bytes = code.to_be_bytes();
        let prefix = code_bytes
            .iter()
            .skip(4_usize.saturating_sub(usize::from(self.len)));
        let mut suffix_count: u64 = bounds
            .clone()
            .map(|(low, high)| u64::from(high.saturating_sub(low)) + 1)
            .product();
        let mut count = 0;
        for ((low, high), &byte) in bounds.zip(prefix) {
            suffix_count /= u64::from(high.saturating_sub(low)) + 1;
            let smaller = u64::from(byte.clamp(low, high).saturating_sub(low));
            count += smaller * suffix_count;
            if byte > high {
                return count + suffix_count;
            }
            if byte < low {
                return count;
            }
        }
        count + 1
    }
}

#[derive(Clone, Copy, Debug)]
struct Interval {
    end: u32,
    first_cid: u32,
    sequential: bool,
}

impl Interval {
    fn cid(self, offset: u32) -> Option<u32> {
        self.first_cid
            .checked_add(if self.sequential { offset } else { 0 })
    }
}

#[derive(Debug, Default)]
struct Ranges([BTreeMap<u32, Interval>; 4]);

impl Ranges {
    fn get(&self, len: u8, code: u32) -> Option<u32> {
        let (start, interval) = self
            .0
            .get(usize::from(len.checked_sub(1)?))?
            .range(..=code)
            .next_back()?;
        if code > interval.end {
            return None;
        }
        interval.cid(code.checked_sub(*start)?)
    }

    // Later assignments override earlier ones. Preserve the portions of old
    // intervals outside the new assignment, without expanding individual codes.
    fn insert(&mut self, len: u8, first: u32, interval: Interval) -> Option<()> {
        let ranges = self.0.get_mut(usize::from(len.checked_sub(1)?))?;
        if let Some((&start, &previous)) = ranges.range(..first).next_back()
            && previous.end >= first
        {
            ranges.insert(
                start,
                Interval {
                    end: first.checked_sub(1)?,
                    ..previous
                },
            );
            preserve_tail(ranges, start, previous, interval.end)?;
        }
        while let Some((&start, &previous)) = ranges.range(first..=interval.end).next() {
            ranges.remove(&start);
            preserve_tail(ranges, start, previous, interval.end)?;
        }
        ranges.insert(first, interval);
        Some(())
    }
}

fn preserve_tail(
    ranges: &mut BTreeMap<u32, Interval>,
    start: u32,
    old: Interval,
    end: u32,
) -> Option<()> {
    if old.end > end {
        let tail = end.checked_add(1)?;
        ranges.insert(
            tail,
            Interval {
                first_cid: old.cid(tail.checked_sub(start)?)?,
                ..old
            },
        );
    }
    Some(())
}

#[derive(Clone, Copy, Debug)]
enum Token<'a> {
    Word(&'a [u8]),
    Name(&'a [u8]),
    Code(Code),
    Opaque,
}

struct Lexer<'a> {
    remaining: &'a [u8],
}

impl<'a> Lexer<'a> {
    fn token(&mut self) -> Option<Token<'a>> {
        self.skip_layout();
        let (&first, tail) = self.remaining.split_first()?;
        match first {
            b'(' => {
                self.remaining = skip_string(tail)?;
                Some(Token::Opaque)
            }
            b'<' if tail.first() != Some(&b'<') => {
                let end = tail.iter().position(|byte| *byte == b'>')?;
                let text = tail.get(..end)?;
                self.remaining = tail.get(end.checked_add(1)?..)?;
                Some(Token::Code(hex_code(text).unwrap_or(Code {
                    bytes: [0; 4],
                    len: 0,
                })))
            }
            b'/' => {
                self.remaining = tail;
                Some(Token::Name(self.word()?))
            }
            b'<' | b'>' | b'[' | b']' | b'{' | b'}' => {
                self.remaining = tail;
                if matches!(first, b'<' | b'>') && self.remaining.first() == Some(&first) {
                    self.remaining = self.remaining.get(1..)?;
                }
                Some(Token::Word(match first {
                    b'{' => b"{",
                    b'}' => b"}",
                    _ => b"",
                }))
            }
            _ => self.word().map(Token::Word),
        }
    }

    fn word(&mut self) -> Option<&'a [u8]> {
        let end = self
            .remaining
            .iter()
            .position(|byte| whitespace(*byte) || b"()<>[]{}/%".contains(byte))
            .unwrap_or(self.remaining.len());
        if end == 0 {
            return None;
        }
        let (word, tail) = self.remaining.split_at_checked(end)?;
        self.remaining = tail;
        Some(word)
    }

    fn skip_layout(&mut self) {
        loop {
            let start = self
                .remaining
                .iter()
                .position(|byte| !whitespace(*byte))
                .unwrap_or(self.remaining.len());
            self.remaining = self.remaining.get(start..).unwrap_or_default();
            if self.remaining.first() != Some(&b'%') {
                break;
            }
            let newline = self
                .remaining
                .iter()
                .position(|byte| matches!(byte, b'\n' | b'\r'));
            self.remaining = newline
                .and_then(|offset| self.remaining.get(offset..))
                .unwrap_or_default();
        }
    }

    fn next_code(&mut self) -> Option<Code> {
        match self.token()? {
            Token::Code(code) if code.len != 0 => Some(code),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<u32> {
        match self.token()? {
            Token::Word(word) => number(word),
            _ => None,
        }
    }
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn skip_string(mut tail: &[u8]) -> Option<&[u8]> {
    let mut depth = 1_u32;
    while let Some((&byte, next)) = tail.split_first() {
        tail = next;
        match byte {
            b'\\' => tail = tail.get(1..)?,
            b'(' => depth = depth.checked_add(1)?,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(tail);
                }
            }
            _ => {}
        }
    }
    None
}

fn hex_code(input: &[u8]) -> Option<Code> {
    let mut bytes = [0_u8; 4];
    let mut digits = 0_u8;
    for byte in input.iter().copied().filter(|byte| !whitespace(*byte)) {
        let value = match byte {
            b'0'..=b'9' => byte.checked_sub(b'0')?,
            b'a'..=b'f' => byte.checked_sub(b'a')?.checked_add(10)?,
            b'A'..=b'F' => byte.checked_sub(b'A')?.checked_add(10)?,
            _ => return None,
        };
        let target = bytes.get_mut(usize::from(digits / 2))?;
        *target |= if digits.is_multiple_of(2) {
            value << 4
        } else {
            value
        };
        digits = digits.checked_add(1)?;
    }
    (digits != 0).then_some(Code {
        bytes,
        len: digits.div_ceil(2),
    })
}

fn number(word: &[u8]) -> Option<u32> {
    std::str::from_utf8(word).ok()?.parse().ok()
}

#[derive(Clone, Copy, Debug)]
enum Block {
    Space,
    CidChar,
    CidRange,
    NotdefChar,
    NotdefRange,
}

impl Block {
    fn operator(word: &[u8]) -> Option<(Self, &'static [u8])> {
        match word {
            b"begincodespacerange" => Some((Self::Space, b"endcodespacerange")),
            b"begincidchar" => Some((Self::CidChar, b"endcidchar")),
            b"begincidrange" => Some((Self::CidRange, b"endcidrange")),
            b"beginnotdefchar" => Some((Self::NotdefChar, b"endnotdefchar")),
            b"beginnotdefrange" => Some((Self::NotdefRange, b"endnotdefrange")),
            _ => None,
        }
    }

    fn insert(
        self,
        cmap: &mut CMap,
        lexer: &mut Lexer<'_>,
        pending_notdef: &mut Vec<(u8, u32, Interval)>,
    ) -> Option<()> {
        let start = lexer.next_code()?;
        let end = if matches!(self, Self::Space | Self::CidRange | Self::NotdefRange) {
            lexer.next_code()?
        } else {
            start
        };
        if matches!(self, Self::Space) {
            let space = Space::from_codes(start, end)?;
            if cmap.spaces.len() >= MAX_SPACE_RANGES
                || cmap.spaces.iter().any(|other| space.overlaps_prefix(other))
            {
                return None;
            }
            cmap.spaces.push(space);
            return Some(());
        }

        let first = start.value();
        let last = end.value();
        if start.len != end.len || first > last {
            return None;
        }
        if !cmap.inherits {
            let covered: u64 = cmap
                .spaces
                .iter()
                .filter(|space| space.len == start.len)
                .map(|space| {
                    space.count_through(last)
                        - first
                            .checked_sub(1)
                            .map_or(0, |before| space.count_through(before))
                })
                .sum();
            if covered != u64::from(last) - u64::from(first) + 1 {
                return None;
            }
        }
        let interval = Interval {
            end: last,
            first_cid: lexer.number()?,
            sequential: matches!(self, Self::CidRange),
        };
        if matches!(self, Self::NotdefChar | Self::NotdefRange) {
            pending_notdef.push((start.len, first, interval));
            return Some(());
        }
        let maximum = interval.cid(last.checked_sub(first)?)?;
        cmap.maximal_cid = cmap.maximal_cid.max(maximum);
        cmap.mappings.insert(start.len, first, interval)
    }
}

fn parse(bytes: &[u8], inherits: bool) -> Option<CMap> {
    let mut lexer = Lexer { remaining: bytes };
    let mut cmap = CMap {
        inherits,
        ..CMap::default()
    };
    let mut pending_notdef = Vec::new();
    let mut in_body = false;
    let mut finished = false;
    let mut previous = None;
    let mut mappings = 0_usize;
    while let Some(token) = lexer.token() {
        match token {
            Token::Word(b"usecmap") if in_body => cmap.inherits = true,
            Token::Word(b"begincmap") if !in_body && !finished => in_body = true,
            Token::Word(b"endcmap") if in_body => {
                in_body = false;
                finished = true;
            }
            Token::Word(b"begincmap" | b"endcmap" | b"{" | b"}") => return None,
            Token::Name(key @ (b"WMode" | b"CMapType")) if in_body => {
                let value = lexer.number()?;
                if !matches!(lexer.token()?, Token::Word(b"def")) {
                    return None;
                }
                if key == b"WMode" {
                    cmap.writing_mode = value;
                } else if value == 2 {
                    cmap.unicode = true;
                }
            }
            Token::Word(b"beginbfchar" | b"beginbfrange") if in_body => {
                cmap.unicode = true;
            }
            Token::Word(word) if in_body => {
                if let Some((block, end)) = Block::operator(word) {
                    let count = previous.filter(|count| *count <= MAX_BLOCK_ENTRIES)?;
                    mappings = mappings.checked_add(usize::try_from(count).ok()?)?;
                    if mappings > MAX_MAPPING_RANGES {
                        return None;
                    }
                    for _ in 0..count {
                        block.insert(&mut cmap, &mut lexer, &mut pending_notdef)?;
                    }
                    if !matches!(lexer.token()?, Token::Word(word) if word == end) {
                        return None;
                    }
                } else if !cmap_word(word) {
                    return None;
                }
            }
            _ => {}
        }
        previous = if let Token::Word(word) = token {
            number(word)
        } else {
            None
        };
    }
    lexer.skip_layout();
    for (len, start, interval) in pending_notdef.into_iter().rev() {
        cmap.notdef.insert(len, start, interval)?;
    }
    (finished && !in_body && lexer.remaining.is_empty()).then_some(cmap)
}

fn cmap_word(word: &[u8]) -> bool {
    matches!(
        word,
        b"" | b"dict"
            | b"dup"
            | b"begin"
            | b"end"
            | b"def"
            | b"readonly"
            | b"currentdict"
            | b"endbfchar"
            | b"endbfrange"
    ) || (!word.is_empty()
        && word
            .iter()
            .all(|byte| byte.is_ascii_digit() || *byte == b'.'))
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::{Result, ValidationError};

    #[test]
    fn test_should_map_mixed_width_codes_and_allow_later_overrides() -> Result<()> {
        let source = br"begincmap
/WMode 0 def
2 begincodespacerange
<00> <7f>
<8100> <81ff>
endcodespacerange
2 begincidrange
<41> <43> 100
<8100> <81ff> 300
endcidrange
1 begincidchar
<42> 7
endcidchar
1 beginnotdefrange
<50> <57> 29
endnotdefrange
endcmap";
        let map = parse(source, false).ok_or(ValidationError::SubsystemUnavailable {
            subsystem: "testCmap",
        })?;
        let mut input = b"ABC\x81\x51PWZ".as_slice();
        for cid in [100, 7, 102, 381, 29, 29, 0] {
            assert_eq!(
                map.next_cid(&mut input)
                    .map(|mapping| mapping.preferred.unwrap_or(mapping.fallback)),
                Some(cid)
            );
        }
        assert_eq!(input, b"");
        assert_eq!(map.maximal_cid(), Some(555));
        assert_eq!(map.writing_mode(), 0);
        Ok(())
    }

    #[test]
    fn test_should_preserve_unaffected_ranges_when_overwriting_multiple_segments() -> Result<()> {
        let map = parse(
            br"begincmap
1 begincodespacerange <00> <ff> endcodespacerange
3 begincidrange
<10> <20> 100
<25> <30> 200
<15> <27> 300
endcidrange
endcmap",
            false,
        )
        .ok_or(ValidationError::SubsystemUnavailable {
            subsystem: "testCmap",
        })?;
        let mut input = b"\x10\x14\x15\x27\x28\x30".as_slice();
        for cid in [100, 104, 300, 318, 203, 211] {
            assert_eq!(
                map.next_cid(&mut input)
                    .and_then(|mapping| mapping.preferred),
                Some(cid)
            );
        }
        Ok(())
    }

    #[test]
    fn test_should_reject_ambiguous_codespaces_and_out_of_space_mappings() {
        for input in [
            b"begincmap 2 begincodespacerange <00> <ff> <1000> <10ff> endcodespacerange endcmap".as_slice(),
            b"begincmap 1 begincidchar <41> endcidchar endcmap",
            b"begincmap 1 begincidrange <00000000> <ffffffff> 1 endcidrange endcmap",
            b"begincmap 1 begincodespacerange <0130> <0240> endcodespacerange 1 begincidrange <0140> <0230> 1 endcidrange endcmap",
            b"begincmap 1 begincodespacerange <10> <40> endcodespacerange 1 begincidrange <0f> <20> 1 endcidrange endcmap",
            b"begincmap { /Identity-H usecmap } exec endcmap",
            b"begincmap 101 begincidchar endcidchar endcmap",
        ] {
            assert!(parse(input, false).is_none());
        }
    }

    #[test]
    fn test_should_keep_out_of_range_cids_available_for_validation() -> Result<()> {
        let map = parse(b"begincmap 1 begincodespacerange <00> <ff> endcodespacerange 1 begincidchar <41> 65536 endcidchar endcmap", false)
            .ok_or(ValidationError::SubsystemUnavailable { subsystem: "testCmap" })?;
        assert_eq!(map.maximal_cid(), Some(65_536));
        let mut input = b"A".as_slice();
        assert_eq!(
            map.next_cid(&mut input)
                .and_then(|mapping| mapping.preferred),
            Some(65_536)
        );
        Ok(())
    }

    #[test]
    fn test_should_preserve_preferred_and_first_fallback_cids() -> Result<()> {
        let map = parse(
            br"begincmap
1 begincodespacerange <00> <ff> endcodespacerange
1 begincidchar <41> 37 endcidchar
1 beginnotdefrange <40> <42> 500 endnotdefrange
1 beginnotdefchar <41> 900 endnotdefchar
endcmap",
            false,
        )
        .ok_or(ValidationError::SubsystemUnavailable {
            subsystem: "testCmap",
        })?;
        let mut input = b"A".as_slice();
        assert_eq!(
            map.next_cid(&mut input),
            Some(super::CidSelection {
                preferred: Some(37),
                fallback: 500
            })
        );
        assert_eq!(map.maximal_cid(), Some(37));
        Ok(())
    }

    #[test]
    fn test_should_retain_local_mode_and_withhold_cids_for_inherited_maps() -> Result<()> {
        let map = parse(b"begincmap /Identity-H usecmap /WMode 1 def endcmap", false).ok_or(
            ValidationError::SubsystemUnavailable {
                subsystem: "testCmap",
            },
        )?;
        assert!(!map.supports_horizontal_cids());
        assert_eq!(map.writing_mode(), 1);
        assert_eq!(map.maximal_cid(), None);
        let map = parse(
            b"begincmap /Identity-H usecmap 1 begincidchar <0041> 65536 endcidchar endcmap",
            false,
        )
        .ok_or(ValidationError::SubsystemUnavailable {
            subsystem: "testCmap",
        })?;
        assert_eq!(map.maximal_cid(), Some(65_536));
        Ok(())
    }
}
