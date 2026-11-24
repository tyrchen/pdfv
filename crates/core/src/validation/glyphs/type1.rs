//! Character names and advances from PDF-embedded Adobe Type 1 programs.

use std::collections::{BTreeMap, BTreeSet};

use read_fonts::ps::PredefinedEncoding;

use super::{sfnt, simple::Encoding};
use crate::{
    CosObject, ParsedDocument, ResourceLimits, Result, StreamObject, validation::semantic_names,
};

const EEXEC_SEED: u16 = 55665;
const GLYPH_SEED: u16 = 4330;
const MAX_WIDTH_BYTES: u16 = 256;
const MAX_WIDTH_SUBR_DEPTH: u8 = 16;

type Advances = BTreeMap<Box<[u8]>, Option<f64>>;

#[derive(Debug)]
pub(super) struct Program {
    encoding: BuiltinEncoding,
    advances: Advances,
}

impl Program {
    pub(super) fn read(
        document: &ParsedDocument,
        stream: &StreamObject,
        limits: &ResourceLimits,
    ) -> Result<Option<Self>> {
        let length = |name| match semantic_names::resolve(document, stream.dictionary.get(name)) {
            Some(CosObject::Integer(value)) => usize::try_from(*value).ok(),
            _ => None,
        };
        let (Some(header), Some(encrypted)) = (length("Length1"), length("Length2")) else {
            return Ok(None);
        };
        Ok(sfnt::decode(stream, limits)?
            .and_then(|data| Self::parse(&data, header, encrypted, limits)))
    }

    fn parse(
        data: &[u8],
        header_len: usize,
        encrypted_len: usize,
        limits: &ResourceLimits,
    ) -> Option<Self> {
        let (header, tail) = data.split_at_checked(header_len)?;
        let encrypted = tail.get(..encrypted_len)?;
        let header_tokens = Tokens::new(header);
        let encoding = BuiltinEncoding::read(header_tokens.clone())?;
        let matrix = font_matrix_scale(header_tokens)?;
        // ISO 32000-1 §9.9 defines the encrypted segment as binary, even
        // when the first ciphertext bytes happen to look like hex digits.
        let private = decrypt(encrypted, EEXEC_SEED).skip(4).collect::<Vec<_>>();
        let advances = read_charstrings(&private, matrix, limits)?;
        Some(Self { encoding, advances })
    }

    pub(super) fn embedded_name(&self, code: u8) -> &[u8] {
        self.encoding.name(code)
    }

    pub(super) fn glyph_name<'a>(&'a self, encoding: &'a Encoding, code: u8) -> &'a [u8] {
        encoding
            .name(code)
            .unwrap_or_else(|| self.embedded_name(code))
    }

    pub(super) fn width(&self, name: &[u8]) -> Option<f64> {
        *self
            .advances
            .get(name)
            .or_else(|| self.advances.get(b".notdef".as_slice()))?
    }

    pub(super) fn glyph_names(&self) -> impl Iterator<Item = &[u8]> {
        self.advances.keys().map(Box::as_ref)
    }

    pub(super) fn contains(&self, name: &[u8]) -> bool {
        self.advances.contains_key(name)
    }
}

#[derive(Debug, Default)]
struct BuiltinEncoding {
    standard: bool,
    names: BTreeMap<u8, Box<[u8]>>,
}

impl BuiltinEncoding {
    fn read(mut tokens: Tokens<'_>) -> Option<Self> {
        tokens.find(|token| *token == b"/Encoding")?;
        let first = tokens.next()?;
        let mut encoding = Self {
            standard: first == b"StandardEncoding",
            names: BTreeMap::new(),
        };
        if first == b"[" {
            for code in u8::MIN..=u8::MAX {
                let token = tokens.next()?;
                if token == b"]" {
                    return Some(encoding);
                }
                encoding.names.insert(code, glyph_name(token)?.into());
            }
            return (tokens.next()? == b"]").then_some(encoding);
        }
        if !encoding.standard
            && (number::<u16>(first).is_none_or(|size| size > 256) || tokens.next()? != b"array")
        {
            return None;
        }
        let mut closed = false;
        while let Some(token) = tokens.next() {
            match token {
                b"def" | b"eexec" => {
                    closed = token == b"def";
                    break;
                }
                b"dup" => {
                    let Some(code) = tokens.peek().and_then(number::<u8>) else {
                        continue;
                    };
                    tokens.next()?;
                    let name = glyph_name(tokens.next()?)?;
                    if tokens.next()? != b"put" {
                        return None;
                    }
                    encoding.names.insert(code, name.into());
                }
                _ => {}
            }
        }
        closed.then_some(encoding)
    }

    fn name(&self, code: u8) -> &[u8] {
        self.names.get(&code).map_or_else(
            || {
                if self.standard {
                    PredefinedEncoding::Standard.name(code).as_bytes()
                } else {
                    b".notdef"
                }
            },
            Box::as_ref,
        )
    }
}

fn font_matrix_scale(mut tokens: Tokens<'_>) -> Option<f64> {
    if tokens.find(|token| *token == b"/FontMatrix").is_none() {
        return Some(1.0);
    }
    if !matches!(tokens.next()?, b"[" | b"{") {
        return None;
    }
    let scale = number::<f64>(tokens.next()?)? * 1000.0;
    scale.is_finite().then_some(scale)
}

fn decrypt(data: &[u8], seed: u16) -> impl Iterator<Item = u8> + '_ {
    data.iter()
        .copied()
        .scan(seed, |state, byte| Some(decrypt_byte(state, byte)))
}

fn decrypt_byte(state: &mut u16, byte: u8) -> u8 {
    let [high, _] = state.to_be_bytes();
    *state = state
        .wrapping_add(u16::from(byte))
        .wrapping_mul(52845)
        .wrapping_add(22719);
    byte ^ high
}

#[derive(Debug)]
struct PrivateDict<'a> {
    len_iv: Option<usize>,
    subrs: BTreeMap<u16, &'a [u8]>,
    readers: BTreeSet<&'a [u8]>,
    definitions: BTreeSet<&'a [u8]>,
}

impl<'a> PrivateDict<'a> {
    fn read(tokens: &mut Tokens<'a>, limits: &ResourceLimits) -> Option<Self> {
        let mut private = Self {
            len_iv: Some(4),
            subrs: BTreeMap::new(),
            readers: BTreeSet::from([b"RD".as_slice(), b"-|".as_slice()]),
            definitions: BTreeSet::from([b"ND".as_slice(), b"|-".as_slice(), b"def".as_slice()]),
        };
        let mut subr_slots = 0_u16;
        let mut previous: [Option<&[u8]>; 3] = [None; 3];
        let mut depth = 0_u32;
        while let Some(token) = tokens.next() {
            let [prefix, index, length] = previous;
            if token == b"{" {
                if depth == 0
                    && let Some(name) = length.and_then(glyph_name)
                {
                    private.read_procedure(name, tokens.clone(), limits)?;
                }
                depth = depth.checked_add(1)?;
                if depth > limits.max_object_depth {
                    return None;
                }
            } else if token == b"}" {
                depth = depth.checked_sub(1)?;
            } else if depth == 0 {
                match token {
                    b"/CharStrings" => return Some(private),
                    b"/lenIV" => {
                        private.len_iv = match number::<i32>(tokens.next()?)? {
                            -1 => None,
                            count => Some(usize::try_from(count).ok()?),
                        }
                    }
                    b"/Subrs" => {
                        subr_slots = number(tokens.next()?)?;
                        if u64::from(subr_slots) > limits.max_array_len
                            || tokens.next()? != b"array"
                        {
                            return None;
                        }
                    }
                    _ if private.readers.contains(token)
                        && length.and_then(number::<usize>).is_some() =>
                    {
                        let bytes = tokens.binary(length.and_then(number)?)?;
                        if prefix == Some(b"dup")
                            && let Some(index) = index.and_then(number::<u16>)
                            && (index >= subr_slots || private.subrs.insert(index, bytes).is_some())
                        {
                            return None;
                        }
                    }
                    _ => {}
                }
            }
            previous = [index, length, Some(token)];
        }
        None
    }

    fn read_procedure(
        &mut self,
        name: &'a [u8],
        mut probe: Tokens<'a>,
        limits: &ResourceLimits,
    ) -> Option<()> {
        let names = match probe.next()? {
            b"string"
                if probe.next() == Some(b"currentfile")
                    && probe.next() == Some(b"exch")
                    && probe.next() == Some(b"readstring")
                    && probe.next() == Some(b"pop")
                    && probe.next() == Some(b"}") =>
            {
                &mut self.readers
            }
            b"noaccess" if probe.next() == Some(b"def") && probe.next() == Some(b"}") => {
                &mut self.definitions
            }
            b"def" if probe.next() == Some(b"}") => &mut self.definitions,
            _ => return Some(()),
        };
        names.insert(name);
        (u64::try_from(names.len()).ok()? <= limits.max_dict_entries).then_some(())
    }

    fn width(&self, bytes: &[u8]) -> Option<f64> {
        let mut machine = WidthMachine {
            private: self,
            stack: Vec::with_capacity(4),
            remaining: MAX_WIDTH_BYTES,
        };
        machine.run(bytes, 0)?.into_width()
    }
}

fn read_charstrings(data: &[u8], matrix: f64, limits: &ResourceLimits) -> Option<Advances> {
    let mut tokens = Tokens::new(data);
    let private = PrivateDict::read(&mut tokens, limits)?;
    let count: u64 = number(tokens.next()?)?;
    if count == 0 || count > limits.max_dict_entries || tokens.next()? != b"dict" {
        return None;
    }
    let first = tokens.next()?;
    if first == b"dup" {
        if tokens.next()? != b"begin" {
            return None;
        }
    } else if first != b"begin" {
        return None;
    }
    let mut advances = BTreeMap::new();
    while let Some(token) = tokens.next() {
        if token == b"end" {
            return (!advances.is_empty()).then_some(advances);
        }
        if u64::try_from(advances.len()).ok()? >= count {
            return None;
        }
        let name = glyph_name(token)?;
        let length = number(tokens.next()?)?;
        if !private.readers.contains(tokens.next()?) {
            return None;
        }
        let program = tokens.binary(length)?;
        let terminator = tokens.next()?;
        let terminator = if terminator == b"noaccess" {
            tokens.next()?
        } else {
            terminator
        };
        if !private.definitions.contains(terminator) {
            return None;
        }
        if advances
            .insert(
                name.into(),
                private
                    .width(program)
                    .map(|width| width * matrix)
                    .filter(|width| width.is_finite()),
            )
            .is_some()
        {
            return None;
        }
    }
    None
}

#[derive(Debug)]
enum WidthStep {
    Width(f64),
    Return,
}

impl WidthStep {
    fn into_width(self) -> Option<f64> {
        if let Self::Width(width) = self {
            Some(width)
        } else {
            None
        }
    }
}

struct WidthMachine<'a, 'data> {
    private: &'a PrivateDict<'data>,
    stack: Vec<f64>,
    remaining: u16,
}

impl WidthMachine<'_, '_> {
    fn run(&mut self, data: &[u8], depth: u8) -> Option<WidthStep> {
        if depth > MAX_WIDTH_SUBR_DEPTH {
            return None;
        }
        let mut data = data.iter().copied();
        let mut state = self.private.len_iv.map(|_| GLYPH_SEED);
        let next = |remaining: &mut u16,
                    state: &mut Option<u16>,
                    data: &mut std::iter::Copied<std::slice::Iter<'_, u8>>| {
            *remaining = remaining.checked_sub(1)?;
            let byte = data.next()?;
            Some(
                state
                    .as_mut()
                    .map_or(byte, |state| decrypt_byte(state, byte)),
            )
        };
        for _ in 0..self.private.len_iv.unwrap_or(0) {
            next(&mut self.remaining, &mut state, &mut data)?;
        }
        loop {
            let mut operand_byte = || next(&mut self.remaining, &mut state, &mut data);
            let byte = operand_byte()?;
            let operand = match byte {
                10 => {
                    let index = subr_index(self.stack.pop()?)?;
                    let subr = self.private.subrs.get(&index)?;
                    if let found @ WidthStep::Width(_) = self.run(subr, depth.saturating_add(1))? {
                        return Some(found);
                    }
                    continue;
                }
                11 if depth != 0 => return Some(WidthStep::Return),
                13 => match self.stack.as_slice() {
                    [_, width] => return Some(WidthStep::Width(*width)),
                    _ => return None,
                },
                12 => match operand_byte()? {
                    7 => match self.stack.as_slice() {
                        [_, _, width, _] => return Some(WidthStep::Width(*width)),
                        _ => return None,
                    },
                    12 => {
                        let denominator = self.stack.pop()?;
                        let numerator = self.stack.pop()?;
                        if denominator == 0.0 {
                            return None;
                        }
                        numerator / denominator
                    }
                    _ => return None,
                },
                32..=246 => f64::from(i16::from(byte) - 139),
                247..=250 => {
                    f64::from((i16::from(byte) - 247) * 256 + i16::from(operand_byte()?) + 108)
                }
                251..=254 => {
                    f64::from(-((i16::from(byte) - 251) * 256 + i16::from(operand_byte()?) + 108))
                }
                255 => f64::from(i32::from_be_bytes([
                    operand_byte()?,
                    operand_byte()?,
                    operand_byte()?,
                    operand_byte()?,
                ])),
                _ => return None,
            };
            if self.stack.len() >= 24 || !operand.is_finite() {
                return None;
            }
            self.stack.push(operand);
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the subroutine operand must first be an exact unsigned 16-bit integer"
)]
fn subr_index(value: f64) -> Option<u16> {
    if value.fract() != 0.0 || !(0.0..=f64::from(u16::MAX)).contains(&value) {
        return None;
    }
    Some(value as u16)
}

fn number<T: std::str::FromStr>(token: &[u8]) -> Option<T> {
    std::str::from_utf8(token).ok()?.parse().ok()
}

fn glyph_name(token: &[u8]) -> Option<&[u8]> {
    token.strip_prefix(b"/").filter(|name| !name.is_empty())
}

#[derive(Clone, Debug)]
struct Tokens<'data>(&'data [u8]);

impl<'data> Tokens<'data> {
    fn new(data: &'data [u8]) -> Self {
        Self(data)
    }

    fn peek(&self) -> Option<&'data [u8]> {
        self.clone().next()
    }

    fn binary(&mut self, length: usize) -> Option<&'data [u8]> {
        let (&separator, tail) = self.0.split_first()?;
        if !separator.is_ascii_whitespace() {
            return None;
        }
        let tail = if separator == b'\r' {
            tail.strip_prefix(b"\n").unwrap_or(tail)
        } else {
            tail
        };
        let (bytes, rest) = tail.split_at_checked(length)?;
        self.0 = rest;
        Some(bytes)
    }
}

impl<'data> Iterator for Tokens<'data> {
    type Item = &'data [u8];

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.0 = self.0.trim_ascii_start();
            let (first, tail) = self.0.split_first()?;
            match first {
                b'%' => {
                    self.0 =
                        tail.get(tail.iter().position(|byte| matches!(byte, b'\r' | b'\n'))?..)?;
                }
                b'(' => {
                    let mut depth = 1_u64;
                    let mut escaped = false;
                    let end = tail.iter().position(|byte| {
                        if escaped {
                            escaped = false;
                        } else {
                            match byte {
                                b'\\' => escaped = true,
                                b'(' => depth = depth.saturating_add(1),
                                b')' => depth = depth.saturating_sub(1),
                                _ => {}
                            }
                        }
                        depth == 0
                    })?;
                    self.0 = tail.get(end.saturating_add(1)..)?;
                }
                b'<' if !tail.starts_with(b"<") => {
                    let end = tail.iter().position(|byte| *byte == b'>')?;
                    self.0 = tail.get(end.saturating_add(1)..)?;
                }
                _ => {
                    let length = if self.0.starts_with(b"<<") || self.0.starts_with(b">>") {
                        2
                    } else if is_delimiter(*first) && *first != b'/' {
                        1
                    } else {
                        tail.iter()
                            .position(|byte| byte.is_ascii_whitespace() || is_delimiter(*byte))
                            .map_or(self.0.len(), |end| end.saturating_add(1))
                    };
                    let (token, rest) = self.0.split_at_checked(length)?;
                    self.0 = rest;
                    return Some(token);
                }
            }
        }
    }
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'%'
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{EEXEC_SEED, Program, decrypt_byte, read_charstrings};
    use crate::ResourceLimits;

    #[rstest]
    #[case(*b"ABCD")]
    #[case([0, 255, 7, 61])]
    fn test_should_read_type1_encoding_and_width_from_an_embedded_program(
        #[case] prefix: [u8; 4],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let header = b"%!PS-AdobeFont-1.0\n/FontMatrix [0.001 0 0 0.001 0 0] def\n/Encoding 256 array dup 0 /minus put readonly def\ncurrentfile eexec\n";
        let mut private = b"/Take{string currentfile exch readstring pop}executeonly def\n/Save{noaccess def}executeonly def\n/lenIV -1 def\n/Subrs 1 array\ndup 0 5 Take ".to_vec();
        private.extend([139, 248, 236, 13, 11]);
        private.extend(b" NP\nND\n/CharStrings 1 dict dup begin\n/minus 3 Take ");
        private.extend([139, 10, 14]);
        private.extend(b" Save\nend\n");
        let mut key = EEXEC_SEED;
        let mut data = prefix.to_vec();
        for byte in prefix {
            let _ = decrypt_byte(&mut key, byte);
        }
        let encrypted = private.into_iter().map(|byte| {
            let [high, _] = key.to_be_bytes();
            let encrypted = byte ^ high;
            let _ = decrypt_byte(&mut key, encrypted);
            encrypted
        });
        data.extend(encrypted);
        let bytes = [header.as_slice(), data.as_slice()].concat();
        let program = Program::parse(&bytes, header.len(), data.len(), &ResourceLimits::default())
            .ok_or("Type 1 test program should parse")?;
        assert_eq!(program.embedded_name(0), b"minus");
        assert_eq!(program.embedded_name(1), b".notdef");
        assert_eq!(program.width(b"minus"), Some(600.0));
        assert_eq!(program.glyph_names().collect::<Vec<_>>(), [b"minus"]);
        Ok(())
    }

    #[test]
    fn test_should_bound_recursive_type1_width_subroutines()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut private = b"/lenIV -1 def\n/Subrs 1 array\ndup 0 3 RD ".to_vec();
        private.extend([139, 10, 11]);
        private.extend(b" NP\nND\n/CharStrings 1 dict dup begin\n/A 3 RD ");
        private.extend([139, 10, 14]);
        private.extend(b" ND\nend\n");
        let glyphs = read_charstrings(&private, 1.0, &ResourceLimits::default())
            .ok_or("Type 1 test CharStrings should parse")?;
        assert_eq!(glyphs.get(b"A".as_slice()), Some(&None));
        Ok(())
    }
}
