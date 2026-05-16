//! Tolerant byte-level PDF parser used by the M0 validator.

use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    num::NonZeroU32,
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    BoundedText, ConfigError, Identifier, ObjectKey, ObjectLocation, ParseError, ParseFact,
    PdfVersion, ResourceLimits, Result, StreamFact, ValidationWarning, XrefFact,
};

const HEADER_MARKER: &[u8] = b"%PDF-";
const EOF_MARKER: &[u8] = b"%%EOF";
const STREAM_MARKER: &[u8] = b"stream";
const ENDSTREAM_MARKER: &[u8] = b"endstream";
const ENDOBJ_MARKER: &[u8] = b"endobj";

/// Seekable PDF source accepted by [`Parser`].
pub trait PdfSource: Read + Seek {}

impl<T> PdfSource for T where T: Read + Seek {}

/// M0 PDF parser.
#[derive(Clone, Debug)]
pub struct Parser {
    limits: ResourceLimits,
}

impl Parser {
    /// Creates a parser with the supplied resource limits.
    #[must_use]
    pub fn new(limits: ResourceLimits) -> Self {
        Self { limits }
    }

    /// Parses a seekable PDF source into a tolerant document model.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when input cannot be read, exceeds a resource
    /// limit, or is too malformed for M0 recovery.
    pub fn parse<R: PdfSource>(&self, mut source: R) -> Result<ParsedDocument> {
        let byte_len = source
            .seek(SeekFrom::End(0))
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;
        if byte_len > self.limits.max_file_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_file_bytes",
            }
            .into());
        }
        source
            .rewind()
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;

        let capacity = usize::try_from(byte_len).map_err(|_| ParseError::LimitExceeded {
            limit: "max_file_bytes",
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        source
            .read_to_end(&mut bytes)
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;

        ByteParser::new(bytes, self.limits.clone()).parse_document()
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new(ResourceLimits::default())
    }
}

/// Parsed PDF document produced by [`Parser`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParsedDocument {
    /// Parsed PDF header version.
    pub version: PdfVersion,
    /// Catalog object referenced by the latest trailer, when available.
    pub catalog: Option<ObjectKey>,
    /// Indirect object store.
    pub objects: ObjectStore,
    /// Parsed trailers.
    pub trailers: Vec<Trailer>,
    /// Parser facts retained for validation.
    pub parse_facts: Vec<ParseFact>,
    /// Recoverable parser warnings.
    pub warnings: Vec<ValidationWarning>,
}

impl ParsedDocument {
    /// Returns true when the document trailer declares encryption.
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Encryption {
                    encrypted: true,
                    handler: _
                }
            )
        })
    }
}

/// Indirect object storage keyed by object number and generation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ObjectStore(BTreeMap<ObjectKey, IndirectObject>);

impl ObjectStore {
    /// Inserts an indirect object.
    pub fn insert(&mut self, object: IndirectObject) {
        self.0.insert(object.key, object);
    }

    /// Returns an indirect object by key.
    #[must_use]
    pub fn get(&self, key: &ObjectKey) -> Option<&IndirectObject> {
        self.0.get(key)
    }

    /// Iterates over indirect objects in key order.
    pub fn values(&self) -> impl Iterator<Item = &IndirectObject> {
        self.0.values()
    }

    /// Returns the number of stored objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true when no objects are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Parsed indirect object.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndirectObject {
    /// Object key.
    pub key: ObjectKey,
    /// Byte offset where the indirect object starts.
    pub offset: u64,
    /// Materialized COS object.
    pub object: CosObject,
}

/// Parsed trailer dictionary.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trailer {
    /// Trailer dictionary.
    pub dictionary: Dictionary,
    /// Byte offset where `trailer` was parsed.
    pub offset: u64,
}

/// PDF COS object.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum CosObject {
    /// Null object.
    Null,
    /// Boolean object.
    Boolean(bool),
    /// Integer number.
    Integer(i64),
    /// Real number, stored as finite `f64`.
    Real(f64),
    /// Name object.
    Name(PdfName),
    /// String object.
    String(PdfString),
    /// Array object.
    Array(Vec<CosObject>),
    /// Dictionary object.
    Dictionary(Dictionary),
    /// Stream object.
    Stream(StreamObject),
    /// Indirect reference.
    Reference(ObjectKey),
}

impl CosObject {
    /// Returns this object as a dictionary when it has that shape.
    #[must_use]
    pub fn as_dictionary(&self) -> Option<&Dictionary> {
        match self {
            Self::Dictionary(dictionary) => Some(dictionary),
            Self::Stream(stream) => Some(&stream.dictionary),
            _ => None,
        }
    }
}

/// PDF dictionary.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Dictionary(BTreeMap<PdfName, CosObject>);

impl Dictionary {
    /// Inserts a key/value pair.
    pub fn insert(&mut self, key: PdfName, value: CosObject) {
        self.0.insert(key, value);
    }

    /// Returns a dictionary value by PDF name.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CosObject> {
        self.0.get(&PdfName::from_static(key))
    }

    /// Iterates over dictionary entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&PdfName, &CosObject)> {
        self.0.iter()
    }

    /// Returns the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true when the dictionary is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// PDF name bytes after hash escape decoding.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PdfName(Vec<u8>);

impl PdfName {
    /// Creates a name from already validated bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the byte limit is exceeded.
    pub fn new(bytes: Vec<u8>, limits: &ResourceLimits) -> std::result::Result<Self, ParseError> {
        if bytes.len() > limits.max_name_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_name_bytes",
            });
        }
        Ok(Self(bytes))
    }

    /// Returns the raw name bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns true when the decoded bytes match an ASCII name.
    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        self.0.as_slice() == value.as_bytes()
    }

    fn from_static(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

impl TryFrom<String> for PdfName {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Ok(Self(value.into_bytes()))
    }
}

impl From<PdfName> for String {
    fn from(value: PdfName) -> Self {
        String::from_utf8_lossy(&value.0).into_owned()
    }
}

/// PDF string bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct PdfString(Vec<u8>);

impl PdfString {
    /// Creates a bounded PDF string.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the byte limit is exceeded.
    pub fn new(bytes: Vec<u8>, limits: &ResourceLimits) -> std::result::Result<Self, ParseError> {
        if bytes.len() > limits.max_string_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_string_bytes",
            });
        }
        Ok(Self(bytes))
    }

    /// Returns raw string bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for PdfString {
    type Error = ConfigError;

    fn try_from(value: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Ok(Self(value))
    }
}

impl From<PdfString> for Vec<u8> {
    fn from(value: PdfString) -> Self {
        value.0
    }
}

/// Parsed stream object with raw byte range metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamObject {
    /// Stream dictionary.
    pub dictionary: Dictionary,
    /// Range of raw stream bytes within the input.
    pub raw_range: StreamRange,
    /// Declared stream length.
    pub declared_length: Option<u64>,
    /// Length discovered by scanning to `endstream`.
    pub discovered_length: u64,
    /// Stream filters as name objects.
    pub filters: Vec<PdfName>,
    /// Shared source bytes used for lazy stream decoding.
    #[serde(skip, default = "empty_source")]
    pub raw_source: Arc<[u8]>,
    /// Whether the `stream` keyword is followed by CRLF.
    pub stream_keyword_crlf_compliant: bool,
    /// Whether `endstream` is preceded by an EOL marker.
    pub endstream_keyword_eol_compliant: bool,
}

impl StreamObject {
    /// Returns decoded stream bytes, enforcing `max_stream_decode_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when a filter is unsupported, decompression fails,
    /// or decoded output exceeds the configured limit.
    pub fn decoded_bytes(
        &self,
        limits: &ResourceLimits,
    ) -> std::result::Result<Vec<u8>, ParseError> {
        let raw_start =
            usize::try_from(self.raw_range.start).map_err(|_| ParseError::ArithmeticOverflow {
                context: "stream raw range",
            })?;
        let raw_end =
            usize::try_from(self.raw_range.end).map_err(|_| ParseError::ArithmeticOverflow {
                context: "stream raw range",
            })?;
        let mut current = self
            .raw_source
            .get(raw_start..raw_end)
            .ok_or(ParseError::Malformed {
                message: bounded("stream raw range out of bounds"),
            })?
            .to_vec();
        for filter in &self.filters {
            if filter.matches("FlateDecode") || filter.matches("Fl") {
                current = decode_flate_limited(&current, limits.max_stream_decode_bytes)?;
            } else {
                return Err(ParseError::UnsupportedFilter {
                    filter: BoundedText::unchecked(String::from_utf8_lossy(filter.as_bytes())),
                });
            }
        }
        let decoded_len =
            u64::try_from(current.len()).map_err(|_| ParseError::ArithmeticOverflow {
                context: "decoded stream length",
            })?;
        if decoded_len > limits.max_stream_decode_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_stream_decode_bytes",
            });
        }
        Ok(current)
    }
}

fn empty_source() -> Arc<[u8]> {
    Arc::from(Vec::<u8>::new())
}

/// Raw stream byte range in the source file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamRange {
    /// Inclusive start offset.
    pub start: u64,
    /// Exclusive end offset.
    pub end: u64,
}

struct ByteParser {
    bytes: Arc<[u8]>,
    limits: ResourceLimits,
    pos: usize,
    parse_facts: Vec<ParseFact>,
    warnings: Vec<ValidationWarning>,
}

#[derive(Clone, Copy, Debug)]
enum NumberToken {
    Integer(i64),
    Real(f64),
}

#[derive(Clone, Copy, Debug)]
struct XrefStreamSummary {
    decoded_bytes: usize,
    entries: u64,
    compressed_entries: u64,
}

impl ByteParser {
    fn new(bytes: Vec<u8>, limits: ResourceLimits) -> Self {
        Self {
            bytes: Arc::from(bytes),
            limits,
            pos: 0,
            parse_facts: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn parse_document(mut self) -> Result<ParsedDocument> {
        let (header_offset, version) = self.parse_header()?;
        self.push_fact(ParseFact::Header {
            offset: header_offset,
            version,
            had_leading_bytes: header_offset != 0,
        });

        let mut objects = ObjectStore::default();
        let mut trailers = Vec::new();
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            if self.starts_with(EOF_MARKER) {
                self.parse_post_eof_fact()?;
                break;
            }
            if self.starts_with(b"startxref") {
                self.skip_line();
                continue;
            }
            if self.starts_with(b"xref") {
                self.parse_xref_and_trailer(&mut trailers)?;
                continue;
            }
            if self.starts_with(b"trailer") {
                self.consume_bytes(b"trailer")?;
                self.skip_ws_and_comments();
                let offset = self.offset()?;
                let dictionary = self.parse_dictionary(0)?;
                trailers.push(Trailer { dictionary, offset });
                continue;
            }

            let before = self.pos;
            match self.parse_indirect_object()? {
                Some(object) => {
                    let object_count = u64::try_from(objects.len())
                        .map_err(|_| ParseError::ArithmeticOverflow {
                            context: "object count",
                        })?
                        .checked_add(1)
                        .ok_or(ParseError::ArithmeticOverflow {
                            context: "object count",
                        })?;
                    if object_count > self.limits.max_objects {
                        return Err(ParseError::LimitExceeded {
                            limit: "max_objects",
                        }
                        .into());
                    }
                    objects.insert(object);
                }
                None => {
                    self.pos = before.saturating_add(1);
                }
            }
        }

        self.materialize_stream_backed_structures(&mut objects, &mut trailers)?;

        let catalog = trailers
            .iter()
            .rev()
            .find_map(|trailer| object_ref_from_dictionary(&trailer.dictionary, "Root"));
        for trailer in &trailers {
            if trailer.dictionary.get("Encrypt").is_some() {
                let handler = encryption_handler(&trailer.dictionary);
                self.push_fact(ParseFact::Encryption {
                    encrypted: true,
                    handler,
                });
            }
        }

        Ok(ParsedDocument {
            version,
            catalog,
            objects,
            trailers,
            parse_facts: self.parse_facts,
            warnings: self.warnings,
        })
    }

    fn materialize_stream_backed_structures(
        &mut self,
        objects: &mut ObjectStore,
        trailers: &mut Vec<Trailer>,
    ) -> std::result::Result<(), ParseError> {
        let streams = objects
            .values()
            .filter_map(|object| match &object.object {
                CosObject::Stream(stream) => Some((object.key, object.offset, stream.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut expanded_objects = Vec::new();
        for (key, offset, stream) in streams {
            if matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("XRef"))
            {
                let summary = self.parse_xref_stream(key, &stream)?;
                let decoded_len = u64::try_from(summary.decoded_bytes).map_err(|_| {
                    ParseError::ArithmeticOverflow {
                        context: "decoded xref stream length",
                    }
                })?;
                self.push_fact(ParseFact::Stream {
                    object: key,
                    fact: StreamFact::Decoded { bytes: decoded_len },
                });
                trailers.push(Trailer {
                    dictionary: stream.dictionary.clone(),
                    offset,
                });
                self.push_fact(ParseFact::Xref {
                    section: ObjectLocation {
                        object: Some(key),
                        offset: Some(offset),
                        path: None,
                    },
                    fact: XrefFact::XrefStreamParsed {
                        entries: summary.entries,
                        compressed_entries: summary.compressed_entries,
                    },
                });
            }
            if matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("ObjStm"))
            {
                let decoded = stream.decoded_bytes(&self.limits)?;
                let decoded_len =
                    u64::try_from(decoded.len()).map_err(|_| ParseError::ArithmeticOverflow {
                        context: "decoded stream length",
                    })?;
                self.push_fact(ParseFact::Stream {
                    object: key,
                    fact: StreamFact::Decoded { bytes: decoded_len },
                });
                let mut parsed_objects = self.parse_object_stream(key, &stream, &decoded)?;
                expanded_objects.append(&mut parsed_objects);
                self.push_fact(ParseFact::Xref {
                    section: ObjectLocation {
                        object: Some(key),
                        offset: Some(offset),
                        path: None,
                    },
                    fact: XrefFact::ObjectStreamParsed,
                });
            }
        }
        for object in expanded_objects {
            if objects.get(&object.key).is_none() {
                let next_count =
                    u64::try_from(objects.len()).map_err(|_| ParseError::ArithmeticOverflow {
                        context: "object count",
                    })? + 1;
                if next_count > self.limits.max_objects {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_objects",
                    });
                }
                objects.insert(object);
            }
        }
        Ok(())
    }

    fn parse_xref_stream(
        &self,
        _stream_key: ObjectKey,
        stream: &StreamObject,
    ) -> std::result::Result<XrefStreamSummary, ParseError> {
        let size = non_negative_u64_from_dictionary(&stream.dictionary, "Size")?;
        if size > self.limits.max_objects {
            return Err(ParseError::LimitExceeded {
                limit: "max_objects",
            });
        }
        let widths = xref_widths(&stream.dictionary)?;
        let indexes = xref_indexes(&stream.dictionary, size)?;
        let entry_width = widths
            .iter()
            .try_fold(0_usize, |sum, width| sum.checked_add(*width))
            .ok_or(ParseError::ArithmeticOverflow {
                context: "xref stream entry width",
            })?;
        if entry_width == 0 {
            return Err(ParseError::Malformed {
                message: bounded("xref stream entry width must be non-zero"),
            });
        }
        let decoded = stream.decoded_bytes(&self.limits)?;
        let total_entries = indexes
            .iter()
            .try_fold(0_u64, |sum, (_, count)| sum.checked_add(*count))
            .ok_or(ParseError::ArithmeticOverflow {
                context: "xref stream entries",
            })?;
        if total_entries > self.limits.max_objects {
            return Err(ParseError::LimitExceeded {
                limit: "max_objects",
            });
        }
        let required_bytes = usize::try_from(total_entries)
            .ok()
            .and_then(|entries| entries.checked_mul(entry_width))
            .ok_or(ParseError::ArithmeticOverflow {
                context: "xref stream bytes",
            })?;
        if decoded.len() < required_bytes {
            return Err(ParseError::Malformed {
                message: bounded("xref stream data shorter than declared entries"),
            });
        }

        let mut pos = 0_usize;
        let mut compressed_entries = 0_u64;
        for (_first_object, count) in indexes {
            for _ in 0..count {
                let entry_type = if widths[0] == 0 {
                    1
                } else {
                    read_be_uint(&decoded, &mut pos, widths[0])?
                };
                let _field_two = read_be_uint(&decoded, &mut pos, widths[1])?;
                let _field_three = read_be_uint(&decoded, &mut pos, widths[2])?;
                if entry_type == 2 {
                    compressed_entries = compressed_entries.checked_add(1).ok_or(
                        ParseError::ArithmeticOverflow {
                            context: "compressed xref entries",
                        },
                    )?;
                }
            }
        }
        Ok(XrefStreamSummary {
            decoded_bytes: decoded.len(),
            entries: total_entries,
            compressed_entries,
        })
    }

    fn parse_object_stream(
        &self,
        stream_key: ObjectKey,
        stream: &StreamObject,
        decoded: &[u8],
    ) -> std::result::Result<Vec<IndirectObject>, ParseError> {
        let count_u64 = non_negative_u64_from_dictionary(&stream.dictionary, "N")?;
        if count_u64 > self.limits.max_objects {
            return Err(ParseError::LimitExceeded {
                limit: "max_objects",
            });
        }
        let first = non_negative_usize_from_dictionary(&stream.dictionary, "First")?;
        if first > decoded.len() {
            return Err(ParseError::Malformed {
                message: bounded("object stream first offset exceeds decoded bytes"),
            });
        }
        let count = usize::try_from(count_u64).map_err(|_| ParseError::LimitExceeded {
            limit: "max_objects",
        })?;
        if count > 0 && count > first / 4 {
            return Err(ParseError::Malformed {
                message: bounded("object stream header too short for object count"),
            });
        }
        let mut parser = ByteParser::new(decoded.to_vec(), self.limits.clone());
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            let Some(number) = parser.parse_unsigned_u32()? else {
                return Err(ParseError::Malformed {
                    message: bounded("object stream missing object number"),
                });
            };
            parser.skip_required_ws()?;
            let Some(relative_offset) = parser.parse_unsigned::<usize>()? else {
                return Err(ParseError::Malformed {
                    message: bounded("object stream missing object offset"),
                });
            };
            let Some(number) = NonZeroU32::new(number) else {
                return Err(ParseError::Malformed {
                    message: bounded("object number must be non-zero"),
                });
            };
            headers.push((
                ObjectKey {
                    number,
                    generation: 0,
                },
                relative_offset,
            ));
        }

        let mut objects = Vec::with_capacity(count);
        for (key, relative_offset) in headers {
            let object_pos =
                first
                    .checked_add(relative_offset)
                    .ok_or(ParseError::ArithmeticOverflow {
                        context: "object stream offset",
                    })?;
            if object_pos >= decoded.len() {
                return Err(ParseError::Malformed {
                    message: bounded("object stream object offset exceeds decoded bytes"),
                });
            }
            parser.pos = object_pos;
            let object = parser.parse_object(0)?;
            let offset = u64::try_from(object_pos)
                .ok()
                .and_then(|relative| stream.raw_range.start.checked_add(relative))
                .ok_or(ParseError::ArithmeticOverflow {
                    context: "object stream object offset",
                })?;
            if key == stream_key {
                return Err(ParseError::Malformed {
                    message: bounded("object stream cannot contain itself"),
                });
            }
            objects.push(IndirectObject {
                key,
                offset,
                object,
            });
        }
        Ok(objects)
    }

    fn parse_header(&mut self) -> std::result::Result<(u64, PdfVersion), ParseError> {
        let Some(header_pos) = find_bytes(&self.bytes, HEADER_MARKER, 0) else {
            return Err(ParseError::Malformed {
                message: bounded("missing PDF header"),
            });
        };
        self.pos = header_pos
            .checked_add(HEADER_MARKER.len())
            .ok_or(ParseError::ArithmeticOverflow { context: "header" })?;
        let mut malformed = false;
        let major = if let Some(value) = self.parse_version_digit() {
            value
        } else {
            malformed = true;
            1
        };
        if self.peek_byte() == Some(b'.') {
            self.pos = self.pos.saturating_add(1);
        } else {
            malformed = true;
        }
        let minor = if let Some(value) = self.parse_version_digit() {
            value
        } else {
            malformed = true;
            4
        };
        if malformed {
            self.warnings.push(ValidationWarning::General {
                message: BoundedText::unchecked("malformed PDF header version recovered as 1.4"),
            });
        }
        Ok((
            u64::try_from(header_pos).map_err(|_| ParseError::ArithmeticOverflow {
                context: "header offset",
            })?,
            PdfVersion { major, minor },
        ))
    }

    fn parse_version_digit(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        if byte.is_ascii_digit() {
            self.pos = self.pos.saturating_add(1);
            Some(byte.saturating_sub(b'0'))
        } else {
            None
        }
    }

    fn parse_indirect_object(&mut self) -> std::result::Result<Option<IndirectObject>, ParseError> {
        let start = self.pos;
        let Some(number) = self.parse_unsigned_u32()? else {
            return Ok(None);
        };
        self.skip_required_ws()?;
        let Some(generation) = self.parse_unsigned_u16()? else {
            self.pos = start;
            return Ok(None);
        };
        self.skip_required_ws()?;
        if !self.starts_with(b"obj") {
            self.pos = start;
            return Ok(None);
        }
        self.consume_bytes(b"obj")?;
        let Some(number) = NonZeroU32::new(number) else {
            return Err(ParseError::Malformed {
                message: bounded("object number must be non-zero"),
            });
        };
        let key = ObjectKey { number, generation };
        let object_start = u64::try_from(start).map_err(|_| ParseError::ArithmeticOverflow {
            context: "object offset",
        })?;

        let parsed = self.parse_object(0)?;
        let object = match parsed {
            CosObject::Dictionary(dictionary) if self.peek_stream_marker() => {
                CosObject::Stream(self.parse_stream(key, dictionary)?)
            }
            other => other,
        };
        self.skip_ws_and_comments();
        if self.starts_with(ENDOBJ_MARKER) {
            self.consume_bytes(ENDOBJ_MARKER)?;
        }
        Ok(Some(IndirectObject {
            key,
            offset: object_start,
            object,
        }))
    }

    fn parse_object(&mut self, depth: u32) -> std::result::Result<CosObject, ParseError> {
        if depth > self.limits.max_object_depth {
            return Err(ParseError::LimitExceeded {
                limit: "max_object_depth",
            });
        }
        self.skip_ws_and_comments();
        if self.starts_with(b"<<") {
            return Ok(CosObject::Dictionary(self.parse_dictionary(depth)?));
        }
        if self.starts_with(b"[") {
            return Ok(CosObject::Array(self.parse_array(depth)?));
        }
        match self.peek_byte() {
            Some(b'/') => self.parse_name().map(CosObject::Name),
            Some(b'(') => self.parse_literal_string().map(CosObject::String),
            Some(b'<') => self.parse_hex_string().map(CosObject::String),
            Some(b't') if self.starts_with(b"true") => {
                self.consume_bytes(b"true")?;
                Ok(CosObject::Boolean(true))
            }
            Some(b'f') if self.starts_with(b"false") => {
                self.consume_bytes(b"false")?;
                Ok(CosObject::Boolean(false))
            }
            Some(b'n') if self.starts_with(b"null") => {
                self.consume_bytes(b"null")?;
                Ok(CosObject::Null)
            }
            Some(b'-' | b'+' | b'.' | b'0'..=b'9') => self.parse_number_or_reference(),
            _ => Err(ParseError::Malformed {
                message: BoundedText::unchecked(format!("unexpected token at {}", self.pos)),
            }),
        }
    }

    fn parse_dictionary(&mut self, depth: u32) -> std::result::Result<Dictionary, ParseError> {
        self.consume_bytes(b"<<")?;
        let mut dictionary = Dictionary::default();
        loop {
            self.skip_ws_and_comments();
            if self.starts_with(b">>") {
                self.consume_bytes(b">>")?;
                return Ok(dictionary);
            }
            let key = self.parse_name()?;
            let value = self.parse_object(depth.saturating_add(1))?;
            let next_len =
                u64::try_from(dictionary.len()).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "dictionary length",
                })? + 1;
            if next_len > self.limits.max_dict_entries {
                return Err(ParseError::LimitExceeded {
                    limit: "max_dict_entries",
                });
            }
            dictionary.insert(key, value);
        }
    }

    fn parse_array(&mut self, depth: u32) -> std::result::Result<Vec<CosObject>, ParseError> {
        self.consume_bytes(b"[")?;
        let mut values = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.starts_with(b"]") {
                self.consume_bytes(b"]")?;
                return Ok(values);
            }
            let value = self.parse_object(depth.saturating_add(1))?;
            let next_len =
                u64::try_from(values.len()).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "array length",
                })? + 1;
            if next_len > self.limits.max_array_len {
                return Err(ParseError::LimitExceeded {
                    limit: "max_array_len",
                });
            }
            values.push(value);
        }
    }

    fn parse_name(&mut self) -> std::result::Result<PdfName, ParseError> {
        self.consume_bytes(b"/")?;
        let mut bytes = Vec::new();
        while let Some(byte) = self.peek_byte() {
            if is_delimiter(byte) || is_ws(byte) {
                break;
            }
            self.pos = self.pos.saturating_add(1);
            if byte == b'#' {
                let high = self.next_byte().ok_or(ParseError::Malformed {
                    message: bounded("truncated name escape"),
                })?;
                let low = self.next_byte().ok_or(ParseError::Malformed {
                    message: bounded("truncated name escape"),
                })?;
                let decoded = decode_hex_pair(high, low).ok_or(ParseError::Malformed {
                    message: bounded("invalid name escape"),
                })?;
                bytes.push(decoded);
            } else {
                bytes.push(byte);
            }
            if bytes.len() > self.limits.max_name_bytes {
                return Err(ParseError::LimitExceeded {
                    limit: "max_name_bytes",
                });
            }
        }
        PdfName::new(bytes, &self.limits)
    }

    fn parse_literal_string(&mut self) -> std::result::Result<PdfString, ParseError> {
        self.consume_bytes(b"(")?;
        let mut depth = 1_u32;
        let mut bytes = Vec::new();
        while let Some(byte) = self.next_byte() {
            match byte {
                b'\\' => {
                    let Some(escaped) = self.next_byte() else {
                        return Err(ParseError::Malformed {
                            message: bounded("truncated string escape"),
                        });
                    };
                    bytes.push(match escaped {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        other => other,
                    });
                }
                b'(' => {
                    depth = depth.checked_add(1).ok_or(ParseError::ArithmeticOverflow {
                        context: "string nesting",
                    })?;
                    bytes.push(byte);
                }
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return PdfString::new(bytes, &self.limits);
                    }
                    bytes.push(byte);
                }
                other => bytes.push(other),
            }
            if bytes.len() > self.limits.max_string_bytes {
                return Err(ParseError::LimitExceeded {
                    limit: "max_string_bytes",
                });
            }
        }
        Err(ParseError::Malformed {
            message: bounded("unterminated literal string"),
        })
    }

    fn parse_hex_string(&mut self) -> std::result::Result<PdfString, ParseError> {
        self.consume_bytes(b"<")?;
        let mut nibbles = Vec::new();
        while let Some(byte) = self.peek_byte() {
            if byte == b'>' {
                self.pos = self.pos.saturating_add(1);
                break;
            }
            self.pos = self.pos.saturating_add(1);
            if !is_ws(byte) {
                nibbles.push(byte);
            }
        }
        if nibbles.len() % 2 != 0 {
            nibbles.push(b'0');
        }
        let mut bytes = Vec::with_capacity(nibbles.len() / 2);
        for pair in nibbles.chunks(2) {
            let high = pair.first().copied().ok_or(ParseError::Malformed {
                message: bounded("invalid hex string"),
            })?;
            let low = pair.get(1).copied().ok_or(ParseError::Malformed {
                message: bounded("invalid hex string"),
            })?;
            let decoded = decode_hex_pair(high, low).ok_or(ParseError::Malformed {
                message: bounded("invalid hex string"),
            })?;
            bytes.push(decoded);
        }
        PdfString::new(bytes, &self.limits)
    }

    fn parse_number_or_reference(&mut self) -> std::result::Result<CosObject, ParseError> {
        let first_start = self.pos;
        let first = self.parse_number_token()?;
        if let NumberToken::Integer(first_integer) = first {
            let after_first = self.pos;
            if self.skip_required_ws().is_ok() {
                let second_start = self.pos;
                if let Some(generation) = self.parse_unsigned_u16()?
                    && self.skip_required_ws().is_ok()
                    && self.starts_with(b"R")
                {
                    self.consume_bytes(b"R")?;
                    if let Some(number) =
                        NonZeroU32::new(u32::try_from(first_integer).map_err(|_| {
                            ParseError::Malformed {
                                message: bounded("reference object number out of range"),
                            }
                        })?)
                    {
                        return Ok(CosObject::Reference(ObjectKey { number, generation }));
                    }
                }
                self.pos = second_start;
            }
            self.pos = after_first;
        }
        self.pos = first_start;
        match self.parse_number_token()? {
            NumberToken::Integer(value) => Ok(CosObject::Integer(value)),
            NumberToken::Real(value) => Ok(CosObject::Real(value)),
        }
    }

    fn parse_number_token(&mut self) -> std::result::Result<NumberToken, ParseError> {
        self.skip_ws_and_comments();
        let start = self.pos;
        if matches!(self.peek_byte(), Some(b'+' | b'-')) {
            self.pos = self.pos.saturating_add(1);
        }
        let mut has_dot = false;
        while let Some(byte) = self.peek_byte() {
            if byte == b'.' {
                has_dot = true;
                self.pos = self.pos.saturating_add(1);
            } else if byte.is_ascii_digit() {
                self.pos = self.pos.saturating_add(1);
            } else {
                break;
            }
        }
        let token = self.slice(start, self.pos)?;
        let text = std::str::from_utf8(token).map_err(|_| ParseError::Malformed {
            message: bounded("number is not valid ASCII"),
        })?;
        if has_dot {
            let value = text.parse::<f64>().map_err(|_| ParseError::Malformed {
                message: bounded("invalid real number"),
            })?;
            if !value.is_finite() {
                return Err(ParseError::Malformed {
                    message: bounded("non-finite real number"),
                });
            }
            Ok(NumberToken::Real(value))
        } else {
            let value = text.parse::<i64>().map_err(|_| ParseError::Malformed {
                message: bounded("invalid integer"),
            })?;
            Ok(NumberToken::Integer(value))
        }
    }

    fn parse_stream(
        &mut self,
        key: ObjectKey,
        dictionary: Dictionary,
    ) -> std::result::Result<StreamObject, ParseError> {
        self.skip_ws_and_comments();
        self.consume_bytes(STREAM_MARKER)?;
        let stream_keyword_crlf_compliant = self.starts_with(b"\r\n");
        if self.starts_with(b"\r\n") {
            self.consume_bytes(b"\r\n")?;
        } else if self.starts_with(b"\n") || self.starts_with(b"\r") {
            self.pos = self.pos.saturating_add(1);
        }
        let data_start = self.pos;
        let declared_length = integer_from_dictionary(&dictionary, "Length")
            .and_then(|value| u64::try_from(value).ok());
        if let Some(declared) = declared_length
            && declared > self.limits.max_stream_declared_bytes
        {
            return Err(ParseError::LimitExceeded {
                limit: "max_stream_declared_bytes",
            });
        }

        let declared_end = declared_length
            .and_then(|length| usize::try_from(length).ok())
            .and_then(|length| data_start.checked_add(length));
        let declared_keyword =
            declared_end.and_then(|offset| endstream_after_optional_eol(&self.bytes, offset));
        let (data_end, endstream_pos) = if let (Some(data_end), Some(keyword_pos)) =
            (declared_end, declared_keyword)
        {
            (data_end, keyword_pos)
        } else {
            let max_scan =
                usize::try_from(self.limits.max_stream_declared_bytes).map_err(|_| {
                    ParseError::LimitExceeded {
                        limit: "max_stream_declared_bytes",
                    }
                })?;
            let scan_end = data_start
                .checked_add(max_scan)
                .map_or(self.bytes.len(), |end| end.min(self.bytes.len()));
            let keyword_pos = find_bytes(self.slice(data_start, scan_end)?, ENDSTREAM_MARKER, 0)
                .and_then(|relative| data_start.checked_add(relative))
                .ok_or(ParseError::Malformed {
                    message: bounded("missing endstream"),
                })?;
            (
                trim_eol_before(&self.bytes, data_start, keyword_pos),
                keyword_pos,
            )
        };
        let endstream_keyword_eol_compliant = has_eol_before(&self.bytes, endstream_pos);
        let discovered_length =
            u64::try_from(data_end.saturating_sub(data_start)).map_err(|_| {
                ParseError::ArithmeticOverflow {
                    context: "stream length",
                }
            })?;
        self.pos = endstream_pos;
        self.consume_bytes(ENDSTREAM_MARKER)?;

        let filters = stream_filters(&dictionary);
        self.push_fact(ParseFact::Stream {
            object: key,
            fact: StreamFact::Length {
                declared: declared_length.unwrap_or(discovered_length),
                discovered: discovered_length,
            },
        });
        self.push_fact(ParseFact::Stream {
            object: key,
            fact: StreamFact::KeywordSpacing {
                stream_keyword_crlf_compliant,
                endstream_keyword_eol_compliant,
            },
        });

        Ok(StreamObject {
            dictionary,
            raw_range: StreamRange {
                start: u64::try_from(data_start).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "stream start",
                })?,
                end: u64::try_from(data_end).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "stream end",
                })?,
            },
            declared_length,
            discovered_length,
            filters,
            raw_source: Arc::clone(&self.bytes),
            stream_keyword_crlf_compliant,
            endstream_keyword_eol_compliant,
        })
    }

    fn parse_xref_and_trailer(
        &mut self,
        trailers: &mut Vec<Trailer>,
    ) -> std::result::Result<(), ParseError> {
        let section_offset = self.offset()?;
        self.consume_bytes(b"xref")?;
        let mut compliant = true;
        let mut parsed_entries = 0_u64;
        loop {
            self.skip_ws_and_comments();
            if self.pos >= self.bytes.len() || self.starts_with(b"trailer") {
                break;
            }
            if self.starts_with(b"startxref") || self.starts_with(EOF_MARKER) {
                break;
            }
            let Some(_first_object) = self.parse_unsigned_u32()? else {
                compliant = false;
                self.skip_line();
                continue;
            };
            self.skip_required_ws()?;
            let Some(count) = self.parse_unsigned_u32()? else {
                compliant = false;
                self.skip_line();
                continue;
            };
            self.skip_line();
            for _ in 0..count {
                let line_start = self.pos;
                let offset = self.parse_fixed_digits(10);
                self.skip_xref_spaces();
                let generation = self.parse_fixed_digits(5);
                self.skip_xref_spaces();
                let marker = self.next_byte();
                if offset.is_none()
                    || generation.is_none()
                    || !matches!(marker, Some(b'n' | b'f'))
                    || !line_had_eol(&self.bytes, line_start, self.pos)
                {
                    compliant = false;
                }
                parsed_entries =
                    parsed_entries
                        .checked_add(1)
                        .ok_or(ParseError::ArithmeticOverflow {
                            context: "xref entries",
                        })?;
                if parsed_entries > self.limits.max_objects {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_objects",
                    });
                }
                self.skip_line();
            }
        }
        self.push_fact(ParseFact::Xref {
            section: ObjectLocation {
                object: None,
                offset: Some(section_offset),
                path: None,
            },
            fact: if compliant {
                XrefFact::EolMarkersComply
            } else {
                XrefFact::MalformedClassic
            },
        });
        loop {
            self.skip_ws_and_comments();
            if self.pos >= self.bytes.len() {
                return Ok(());
            }
            if self.starts_with(b"trailer") {
                self.consume_bytes(b"trailer")?;
                self.skip_ws_and_comments();
                let offset = self.offset()?;
                let dictionary = self.parse_dictionary(0)?;
                trailers.push(Trailer { dictionary, offset });
                return Ok(());
            }
            if self.starts_with(b"startxref") || self.starts_with(EOF_MARKER) {
                return Ok(());
            }
            self.skip_line();
        }
    }

    fn parse_post_eof_fact(&mut self) -> std::result::Result<(), ParseError> {
        self.consume_bytes(EOF_MARKER)?;
        let remaining = self
            .bytes
            .len()
            .saturating_sub(self.pos)
            .saturating_sub(count_trailing_ws(self.slice(self.pos, self.bytes.len())?));
        if remaining > 0 {
            self.push_fact(ParseFact::PostEofData {
                bytes: u64::try_from(remaining).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "post eof bytes",
                })?,
            });
        }
        Ok(())
    }

    fn push_fact(&mut self, fact: ParseFact) {
        if self.parse_facts.len() >= self.limits.max_parse_facts {
            if !self
                .warnings
                .iter()
                .any(|warning| matches!(warning, ValidationWarning::ParseFactCapReached { .. }))
            {
                self.warnings.push(ValidationWarning::ParseFactCapReached {
                    cap: self.limits.max_parse_facts,
                });
            }
            return;
        }
        self.parse_facts.push(fact);
    }

    fn peek_stream_marker(&mut self) -> bool {
        let saved = self.pos;
        self.skip_ws_and_comments();
        let found = self.starts_with(STREAM_MARKER);
        self.pos = saved;
        found
    }

    fn parse_unsigned_u32(&mut self) -> std::result::Result<Option<u32>, ParseError> {
        self.parse_unsigned::<u32>()
    }

    fn parse_unsigned_u16(&mut self) -> std::result::Result<Option<u16>, ParseError> {
        self.parse_unsigned::<u16>()
    }

    fn parse_fixed_digits(&mut self, len: usize) -> Option<u64> {
        let end = self.pos.checked_add(len)?;
        let slice = self.bytes.get(self.pos..end)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.pos = end;
        std::str::from_utf8(slice).ok()?.parse::<u64>().ok()
    }

    fn parse_unsigned<T>(&mut self) -> std::result::Result<Option<T>, ParseError>
    where
        T: std::str::FromStr,
    {
        self.skip_ws_and_comments();
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_digit() {
                self.pos = self.pos.saturating_add(1);
            } else {
                break;
            }
        }
        if start == self.pos {
            return Ok(None);
        }
        let token = self.slice(start, self.pos)?;
        let text = std::str::from_utf8(token).map_err(|_| ParseError::Malformed {
            message: bounded("unsigned integer is not ASCII"),
        })?;
        text.parse::<T>()
            .map(Some)
            .map_err(|_| ParseError::Malformed {
                message: bounded("unsigned integer out of range"),
            })
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.peek_byte().is_some_and(is_ws) {
                self.pos = self.pos.saturating_add(1);
            }
            if self.starts_with(EOF_MARKER) {
                break;
            }
            if self.peek_byte() == Some(b'%') {
                self.skip_line();
            } else {
                break;
            }
        }
    }

    fn skip_xref_spaces(&mut self) {
        while matches!(self.peek_byte(), Some(b'\t' | b' ')) {
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn skip_required_ws(&mut self) -> std::result::Result<(), ParseError> {
        let start = self.pos;
        while self.peek_byte().is_some_and(is_ws) {
            self.pos = self.pos.saturating_add(1);
        }
        if self.pos == start {
            return Err(ParseError::Malformed {
                message: bounded("expected whitespace"),
            });
        }
        Ok(())
    }

    fn skip_line(&mut self) {
        while let Some(byte) = self.peek_byte() {
            self.pos = self.pos.saturating_add(1);
            if matches!(byte, b'\n' | b'\r') {
                break;
            }
        }
    }

    fn consume_bytes(&mut self, expected: &[u8]) -> std::result::Result<(), ParseError> {
        if !self.starts_with(expected) {
            return Err(ParseError::Malformed {
                message: BoundedText::unchecked(format!("unexpected token at {}", self.pos)),
            });
        }
        self.pos = self
            .pos
            .checked_add(expected.len())
            .ok_or(ParseError::ArithmeticOverflow { context: "offset" })?;
        Ok(())
    }

    fn starts_with(&self, expected: &[u8]) -> bool {
        self.bytes
            .get(self.pos..)
            .is_some_and(|tail| tail.starts_with(expected))
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos = self.pos.saturating_add(1);
        Some(byte)
    }

    fn slice(&self, start: usize, end: usize) -> std::result::Result<&[u8], ParseError> {
        self.bytes.get(start..end).ok_or(ParseError::Malformed {
            message: bounded("byte range out of bounds"),
        })
    }

    fn offset(&self) -> std::result::Result<u64, ParseError> {
        u64::try_from(self.pos).map_err(|_| ParseError::ArithmeticOverflow { context: "offset" })
    }
}

fn bounded(value: &str) -> BoundedText {
    BoundedText::unchecked(value)
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn decode_hex_pair(high: u8, low: u8) -> Option<u8> {
    let high = decode_hex_digit(high)?;
    let low = decode_hex_digit(low)?;
    Some(high.saturating_mul(16).saturating_add(low))
}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.saturating_sub(b'0')),
        b'a'..=b'f' => Some(byte.saturating_sub(b'a').saturating_add(10)),
        b'A'..=b'F' => Some(byte.saturating_sub(b'A').saturating_add(10)),
        _ => None,
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    haystack
        .get(start..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .and_then(|relative| start.checked_add(relative))
}

fn has_eol_before(bytes: &[u8], pos: usize) -> bool {
    matches!(
        pos.checked_sub(1)
            .and_then(|index| bytes.get(index))
            .copied(),
        Some(b'\n' | b'\r')
    )
}

fn line_had_eol(bytes: &[u8], line_start: usize, _current: usize) -> bool {
    bytes
        .get(line_start..)
        .is_some_and(|line| line.iter().any(|byte| matches!(byte, b'\n' | b'\r')))
}

fn endstream_after_optional_eol(bytes: &[u8], offset: usize) -> Option<usize> {
    if bytes
        .get(offset..)
        .is_some_and(|tail| tail.starts_with(ENDSTREAM_MARKER))
    {
        return Some(offset);
    }
    if bytes
        .get(offset..)
        .is_some_and(|tail| tail.starts_with(b"\r\nendstream"))
    {
        return offset.checked_add(2);
    }
    if bytes
        .get(offset..)
        .is_some_and(|tail| tail.starts_with(b"\nendstream") || tail.starts_with(b"\rendstream"))
    {
        return offset.checked_add(1);
    }
    None
}

fn trim_eol_before(bytes: &[u8], data_start: usize, keyword_pos: usize) -> usize {
    if keyword_pos >= data_start.saturating_add(2)
        && bytes.get(keyword_pos.saturating_sub(2)..keyword_pos) == Some(b"\r\n")
    {
        return keyword_pos.saturating_sub(2);
    }
    if keyword_pos > data_start
        && matches!(
            bytes.get(keyword_pos.saturating_sub(1)).copied(),
            Some(b'\n' | b'\r')
        )
    {
        return keyword_pos.saturating_sub(1);
    }
    keyword_pos
}

fn count_trailing_ws(bytes: &[u8]) -> usize {
    bytes.iter().rev().take_while(|byte| is_ws(**byte)).count()
}

fn integer_from_dictionary(dictionary: &Dictionary, key: &str) -> Option<i64> {
    match dictionary.get(key) {
        Some(CosObject::Integer(value)) => Some(*value),
        _ => None,
    }
}

fn non_negative_usize_from_dictionary(
    dictionary: &Dictionary,
    key: &'static str,
) -> std::result::Result<usize, ParseError> {
    let value = non_negative_u64_from_dictionary(dictionary, key)?;
    usize::try_from(value).map_err(|_| ParseError::Malformed {
        message: BoundedText::unchecked(format!("invalid object stream {key}")),
    })
}

fn non_negative_u64_from_dictionary(
    dictionary: &Dictionary,
    key: &'static str,
) -> std::result::Result<u64, ParseError> {
    let Some(value) = integer_from_dictionary(dictionary, key) else {
        return Err(ParseError::Malformed {
            message: BoundedText::unchecked(format!("missing integer dictionary key {key}")),
        });
    };
    u64::try_from(value).map_err(|_| ParseError::Malformed {
        message: BoundedText::unchecked(format!("invalid non-negative dictionary key {key}")),
    })
}

fn xref_widths(dictionary: &Dictionary) -> std::result::Result<[usize; 3], ParseError> {
    let Some(CosObject::Array(values)) = dictionary.get("W") else {
        return Err(ParseError::Malformed {
            message: bounded("xref stream missing W array"),
        });
    };
    if values.len() != 3 {
        return Err(ParseError::Malformed {
            message: bounded("xref stream W array must have three entries"),
        });
    }
    let mut widths = [0_usize; 3];
    for (index, value) in values.iter().enumerate() {
        let CosObject::Integer(width) = value else {
            return Err(ParseError::Malformed {
                message: bounded("xref stream W entry must be integer"),
            });
        };
        let width = usize::try_from(*width).map_err(|_| ParseError::Malformed {
            message: bounded("xref stream W entry must be non-negative"),
        })?;
        if width > 8 {
            return Err(ParseError::Malformed {
                message: bounded("xref stream W entry exceeds supported width"),
            });
        }
        let Some(slot) = widths.get_mut(index) else {
            return Err(ParseError::Malformed {
                message: bounded("xref stream W index out of bounds"),
            });
        };
        *slot = width;
    }
    Ok(widths)
}

fn xref_indexes(
    dictionary: &Dictionary,
    size: u64,
) -> std::result::Result<Vec<(u64, u64)>, ParseError> {
    let Some(index_object) = dictionary.get("Index") else {
        return Ok(vec![(0, size)]);
    };
    let CosObject::Array(values) = index_object else {
        return Err(ParseError::Malformed {
            message: bounded("xref stream Index must be an array"),
        });
    };
    if values.len() % 2 != 0 {
        return Err(ParseError::Malformed {
            message: bounded("xref stream Index must contain pairs"),
        });
    }
    let mut indexes = Vec::with_capacity(values.len() / 2);
    for pair in values.chunks(2) {
        let first = integer_value(pair.first(), "xref stream Index first")?;
        let count = integer_value(pair.get(1), "xref stream Index count")?;
        let first = u64::try_from(first).map_err(|_| ParseError::Malformed {
            message: bounded("xref stream Index first must be non-negative"),
        })?;
        let count = u64::try_from(count).map_err(|_| ParseError::Malformed {
            message: bounded("xref stream Index count must be non-negative"),
        })?;
        first
            .checked_add(count)
            .ok_or(ParseError::ArithmeticOverflow {
                context: "xref stream Index",
            })?;
        indexes.push((first, count));
    }
    Ok(indexes)
}

fn integer_value(
    value: Option<&CosObject>,
    context: &'static str,
) -> std::result::Result<i64, ParseError> {
    match value {
        Some(CosObject::Integer(value)) => Ok(*value),
        _ => Err(ParseError::Malformed {
            message: BoundedText::unchecked(format!("{context} must be integer")),
        }),
    }
}

fn read_be_uint(
    bytes: &[u8],
    pos: &mut usize,
    width: usize,
) -> std::result::Result<u64, ParseError> {
    let end = pos
        .checked_add(width)
        .ok_or(ParseError::ArithmeticOverflow {
            context: "xref stream field",
        })?;
    let field = bytes.get(*pos..end).ok_or(ParseError::Malformed {
        message: bounded("xref stream field out of bounds"),
    })?;
    let mut value = 0_u64;
    for byte in field {
        value = value
            .checked_mul(256)
            .and_then(|current| current.checked_add(u64::from(*byte)))
            .ok_or(ParseError::ArithmeticOverflow {
                context: "xref stream field",
            })?;
    }
    *pos = end;
    Ok(value)
}

fn object_ref_from_dictionary(dictionary: &Dictionary, key: &str) -> Option<ObjectKey> {
    match dictionary.get(key) {
        Some(CosObject::Reference(value)) => Some(*value),
        _ => None,
    }
}

fn stream_filters(dictionary: &Dictionary) -> Vec<PdfName> {
    match dictionary.get("Filter") {
        Some(CosObject::Name(name)) => vec![name.clone()],
        Some(CosObject::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                CosObject::Name(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn encryption_handler(dictionary: &Dictionary) -> Option<Identifier> {
    let Some(CosObject::Dictionary(encrypt)) = dictionary.get("Encrypt") else {
        return None;
    };
    let Some(CosObject::Name(filter)) = encrypt.get("Filter") else {
        return None;
    };
    let text = String::from_utf8_lossy(filter.as_bytes()).into_owned();
    Identifier::new(text).ok()
}

#[cfg(feature = "flate")]
fn decode_flate_limited(
    bytes: &[u8],
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    use flate2::read::{DeflateDecoder, ZlibDecoder};

    read_limited(
        ZlibDecoder::new(std::io::Cursor::new(bytes)),
        max_decode_bytes,
    )
    .or_else(|_| {
        read_limited(
            DeflateDecoder::new(std::io::Cursor::new(bytes)),
            max_decode_bytes,
        )
    })
}

#[cfg(not(feature = "flate"))]
fn decode_flate_limited(
    _bytes: &[u8],
    _max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    Err(ParseError::UnsupportedFilter {
        filter: BoundedText::unchecked("FlateDecode"),
    })
}

#[cfg(feature = "flate")]
fn read_limited(
    mut reader: impl Read,
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ParseError::StreamDecode {
                message: BoundedText::unchecked(source.to_string()),
            })?;
        if read == 0 {
            return Ok(output);
        }
        let next_len = u64::try_from(output.len())
            .ok()
            .and_then(|len| {
                u64::try_from(read)
                    .ok()
                    .and_then(|read| len.checked_add(read))
            })
            .ok_or(ParseError::ArithmeticOverflow {
                context: "decoded stream length",
            })?;
        if next_len > max_decode_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_stream_decode_bytes",
            });
        }
        let chunk = buffer.get(..read).ok_or(ParseError::Malformed {
            message: bounded("decode buffer range out of bounds"),
        })?;
        output.extend_from_slice(chunk);
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io::Cursor};

    use proptest::prelude::*;
    use rstest::rstest;

    use super::{CosObject, Parser};
    use crate::{ParseFact, ResourceLimits, StreamFact};

    fn minimal_pdf() -> Vec<u8> {
        br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
xref
0 2
0000000000 65535 f 
0000000009 00000 n 
trailer
<< /Root 1 0 R /Size 2 >>
startxref
45
%%EOF
"
        .to_vec()
    }

    fn xref_stream_data(entries: &[(u8, u32, u16)]) -> Vec<u8> {
        let mut data = Vec::with_capacity(entries.len() * 7);
        for (entry_type, field_two, field_three) in entries {
            data.push(*entry_type);
            data.extend(field_two.to_be_bytes());
            data.extend(field_three.to_be_bytes());
        }
        data
    }

    #[test]
    fn test_should_parse_header_and_catalog_from_m0_fixture() -> crate::Result<()> {
        let document = Parser::default().parse(Cursor::new(minimal_pdf()))?;

        assert_eq!(document.version.major, 1);
        assert_eq!(document.version.minor, 7);
        assert!(document.catalog.is_some());
        assert_eq!(document.objects.len(), 1);
        Ok(())
    }

    #[test]
    fn test_should_record_leading_header_bytes() -> crate::Result<()> {
        let mut bytes = b"junk".to_vec();
        bytes.extend(minimal_pdf());

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Header {
                    offset: 4,
                    had_leading_bytes: true,
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_warn_on_malformed_recoverable_header() -> crate::Result<()> {
        let bytes = br"%PDF-x.y
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(!document.warnings.is_empty());
        assert_eq!(document.version, crate::PdfVersion { major: 1, minor: 4 });
        Ok(())
    }

    #[test]
    fn test_should_parse_names_strings_numbers_arrays_and_dictionaries() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Name /A#20B /Title (hello\nworld) /Nums [1 -2 3.5 true false null] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";

        let document = Parser::default().parse(Cursor::new(bytes))?;
        let object =
            document
                .objects
                .values()
                .next()
                .ok_or_else(|| crate::ParseError::MissingObject {
                    message: crate::BoundedText::unchecked("missing object"),
                })?;
        let dictionary =
            object
                .object
                .as_dictionary()
                .ok_or_else(|| crate::ParseError::Malformed {
                    message: crate::BoundedText::unchecked("missing dictionary"),
                })?;

        assert!(
            matches!(dictionary.get("Nums"), Some(CosObject::Array(values)) if values.len() == 6)
        );
        Ok(())
    }

    #[test]
    fn test_should_scan_bad_stream_length_and_emit_facts() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Length 99 >>
stream
abc
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Stream {
                    fact: StreamFact::Length {
                        declared: 99,
                        discovered: 3
                    },
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_parse_xref_stream_as_trailer_source() -> crate::Result<()> {
        let xref_data = xref_stream_data(&[(0, 0, 65_535), (1, 9, 0), (1, 45, 0)]);
        let mut bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
2 0 obj
<< /Type /XRef /Size 3 /W [1 4 2] /Index [0 3] /Length "
            .to_vec();
        bytes.extend(xref_data.len().to_string().as_bytes());
        bytes.extend(
            br" /Root 1 0 R >>
stream
",
        );
        bytes.extend(xref_data);
        bytes.extend(
            br"
endstream
endobj
%%EOF
",
        );

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.catalog.is_some());
        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Xref {
                    fact: crate::XrefFact::XrefStreamParsed { .. },
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_parse_flate_xref_stream_with_compressed_entry() -> Result<(), Box<dyn Error>> {
        use std::io::Write;

        use flate2::{Compression, write::ZlibEncoder};

        let xref_data = xref_stream_data(&[(2, 2, 0), (1, 3, 0)]);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&xref_data)?;
        let compressed = encoder.finish()?;
        let mut bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
2 0 obj
<< /Type /ObjStm /N 0 /First 0 /Length 0 >>
stream
endstream
endobj
3 0 obj
<< /Type /XRef /Size 3 /W [1 4 2] /Index [1 2] /Filter /FlateDecode /Length "
            .to_vec();
        bytes.extend(compressed.len().to_string().as_bytes());
        bytes.extend(
            br" /Root 1 0 R >>
stream
",
        );
        bytes.extend(compressed);
        bytes.extend(
            br"
endstream
endobj
%%EOF
",
        );

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Xref {
                    fact: crate::XrefFact::XrefStreamParsed {
                        entries: 2,
                        compressed_entries: 1
                    },
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_expand_unfiltered_object_stream() -> crate::Result<()> {
        let object_stream = b"1 0 << /Type /Catalog >>";
        let mut bytes = br"%PDF-1.7
2 0 obj
<< /Type /ObjStm /N 1 /First 4 /Length "
            .to_vec();
        bytes.extend(object_stream.len().to_string().as_bytes());
        bytes.extend(
            br" >>
stream
",
        );
        bytes.extend(object_stream);
        bytes.extend(
            br"
endstream
endobj
3 0 obj
<< /Type /XRef /Size 4 /W [1 1 1] /Index [0 0] /Length 0 /Root 1 0 R >>
stream
endstream
endobj
%%EOF
",
        );

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.catalog.is_some());
        assert_eq!(document.objects.len(), 3);
        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Xref {
                    fact: crate::XrefFact::ObjectStreamParsed,
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_decode_flate_object_stream_with_limit() -> Result<(), Box<dyn Error>> {
        use std::io::Write;

        use flate2::{Compression, write::ZlibEncoder};

        let object_stream = b"1 0 << /Type /Catalog >>";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(object_stream)?;
        let compressed = encoder.finish()?;
        let mut bytes = br"%PDF-1.7
2 0 obj
<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length "
            .to_vec();
        bytes.extend(compressed.len().to_string().as_bytes());
        bytes.extend(
            br" >>
stream
",
        );
        bytes.extend(compressed);
        bytes.extend(
            br"
endstream
endobj
3 0 obj
<< /Type /XRef /Size 4 /W [1 1 1] /Index [0 0] /Length 0 /Root 1 0 R >>
stream
endstream
endobj
%%EOF
",
        );

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.catalog.is_some());
        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Stream {
                    fact: StreamFact::Decoded { bytes: 24 },
                    ..
                }
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_enforce_name_limit() {
        let limits = ResourceLimits {
            max_name_bytes: 2,
            ..ResourceLimits::default()
        };
        let bytes = br"%PDF-1.7
1 0 obj
<< /Long /Name >>
endobj
%%EOF
";

        let result = Parser::new(limits).parse(Cursor::new(bytes));

        assert!(result.is_err());
    }

    #[rstest]
    #[case(b"/A", "A")]
    #[case(b"/A#20B", "A B")]
    fn test_should_parse_name_escape_matrix(
        #[case] token: &[u8],
        #[case] expected: &str,
    ) -> crate::Result<()> {
        let mut bytes = b"%PDF-1.7\n1 0 obj\n<< /Name ".to_vec();
        bytes.extend(token);
        bytes.extend(b" >>\nendobj\n%%EOF\n");

        let document = Parser::default().parse(Cursor::new(bytes))?;
        let object =
            document
                .objects
                .values()
                .next()
                .ok_or_else(|| crate::ParseError::MissingObject {
                    message: crate::BoundedText::unchecked("missing object"),
                })?;
        let dictionary =
            object
                .object
                .as_dictionary()
                .ok_or_else(|| crate::ParseError::Malformed {
                    message: crate::BoundedText::unchecked("missing dictionary"),
                })?;

        assert!(
            matches!(dictionary.get("Name"), Some(CosObject::Name(name)) if name.as_bytes() == expected.as_bytes())
        );
        Ok(())
    }

    proptest! {
        #[test]
        fn test_should_not_panic_on_arbitrary_bytes(input in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = Parser::default().parse(Cursor::new(input));
        }
    }
}
