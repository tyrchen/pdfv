//! Tolerant byte-level PDF parser used by the validator.

#[cfg(feature = "decrypt")]
mod encryption;

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    io::{Read, Seek, SeekFrom, Write},
    num::NonZeroU32,
    sync::{Arc, OnceLock},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{
    BoundedText, ConfigError, Identifier, ObjectKey, ObjectLocation, ParseError, ParseFact,
    PasswordSecret, PdfVersion, ResourceLimits, Result, StreamFact, ValidationWarning, XrefFact,
};

const HEADER_MARKER: &[u8] = b"%PDF-";
const EOF_MARKER: &[u8] = b"%%EOF";
const STREAM_MARKER: &[u8] = b"stream";
const ENDSTREAM_MARKER: &[u8] = b"endstream";
const ENDOBJ_MARKER: &[u8] = b"endobj";
const SPILL_SEARCH_CHUNK_BYTES: usize = 8192;

#[allow(
    clippy::disallowed_types,
    reason = "parser source storage is synchronous Read+Seek; async file handles do not fit this \
              API"
)]
type SpillFileHandle = std::fs::File;

/// Seekable PDF source accepted by [`Parser`].
pub trait PdfSource: Read + Seek {}

impl<T> PdfSource for T where T: Read + Seek {}

/// M0 PDF parser.
#[derive(Clone, Debug)]
pub struct Parser {
    limits: ResourceLimits,
    decoder_registry: Option<DecoderRegistry>,
}

impl Parser {
    /// Creates a parser with the supplied resource limits.
    #[must_use]
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            decoder_registry: None,
        }
    }

    /// Creates a parser with explicit resource limits and stream decoders.
    #[must_use]
    pub fn with_decoder_registry(
        limits: ResourceLimits,
        decoder_registry: DecoderRegistry,
    ) -> Self {
        Self {
            limits,
            decoder_registry: Some(decoder_registry),
        }
    }

    /// Parses a seekable PDF source into a tolerant document model.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when input cannot be read, exceeds a resource
    /// limit, or is too malformed for M0 recovery.
    pub fn parse<R: PdfSource>(&self, source: R) -> Result<ParsedDocument> {
        self.parse_with_options(source, ParseOptions::default())
    }

    /// Parses a seekable PDF source with optional password state.
    ///
    /// # Errors
    ///
    /// Returns [`crate::PdfvError`] when input cannot be read, exceeds a resource
    /// limit, or is too malformed for recovery.
    pub fn parse_with_options<R: PdfSource>(
        &self,
        mut source: R,
        options: ParseOptions<'_>,
    ) -> Result<ParsedDocument> {
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

        let storage = SourceStorage::from_source(
            source,
            byte_len,
            self.limits.memory_source_threshold_bytes,
        )?;

        ByteParser::new(
            storage,
            self.limits.clone(),
            self.decoder_registry.clone(),
            options,
        )
        .parse_document()
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new(ResourceLimits::default())
    }
}

fn default_decoder_registry() -> &'static DecoderRegistry {
    static REGISTRY: OnceLock<DecoderRegistry> = OnceLock::new();
    REGISTRY.get_or_init(DecoderRegistry::default)
}

/// Stream decoder extension point used by [`DecoderRegistry`].
pub trait StreamDecoder: fmt::Debug {
    /// Decodes one PDF stream filter under parser resource limits.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the encoded bytes are malformed or the
    /// decoded output exceeds configured limits.
    fn decode(
        &self,
        input: &[u8],
        params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError>;
}

/// Result of applying one stream decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecoderOutput {
    /// Decoded or byte-preserved output bytes.
    pub bytes: Vec<u8>,
    /// Whether the decoder deliberately preserved encoded bytes in metadata mode.
    pub metadata_mode: bool,
}

/// Registry mapping PDF filter names to bounded decoders.
#[derive(Clone)]
pub struct DecoderRegistry {
    decoders: BTreeMap<PdfName, Arc<dyn StreamDecoder + Send + Sync>>,
}

/// Parser-owned storage for source bytes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SourceStorage {
    /// Source bytes are held in memory.
    Memory(Arc<[u8]>),
    /// Source bytes are held in a temporary spill file.
    SpillFile {
        /// Spill file handle.
        file: Arc<SpillFileHandle>,
        /// Total source byte length.
        len: usize,
        /// Temporary path retained for deterministic cleanup on drop.
        path: Arc<tempfile::TempPath>,
    },
}

impl SourceStorage {
    fn from_source<R: PdfSource>(
        mut source: R,
        byte_len: u64,
        memory_threshold: u64,
    ) -> Result<Self> {
        if byte_len <= memory_threshold {
            let capacity = usize::try_from(byte_len).map_err(|_| ParseError::LimitExceeded {
                limit: "max_file_bytes",
            })?;
            let mut bytes = Vec::with_capacity(capacity);
            source
                .read_to_end(&mut bytes)
                .map_err(|source| crate::PdfvError::Io { path: None, source })?;
            return Ok(Self::Memory(Arc::from(bytes)));
        }

        let mut tempfile =
            NamedTempFile::new().map_err(|source| crate::PdfvError::Io { path: None, source })?;
        let copied = std::io::copy(&mut source, &mut tempfile)
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;
        if copied != byte_len {
            return Err(ParseError::Malformed {
                message: bounded("source length changed while spilling"),
            }
            .into());
        }
        tempfile
            .as_file_mut()
            .flush()
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;
        let file = tempfile
            .reopen()
            .map_err(|source| crate::PdfvError::Io { path: None, source })?;
        Ok(Self::SpillFile {
            file: Arc::new(file),
            len: usize::try_from(byte_len).map_err(|_| ParseError::LimitExceeded {
                limit: "max_file_bytes",
            })?,
            path: Arc::new(tempfile.into_temp_path()),
        })
    }

    fn len(&self) -> usize {
        match self {
            Self::Memory(bytes) => bytes.len(),
            Self::SpillFile { len, .. } => *len,
        }
    }

    fn slice(&self, start: usize, end: usize) -> std::result::Result<Cow<'_, [u8]>, ParseError> {
        if start > end || end > self.len() {
            return Err(ParseError::Malformed {
                message: bounded("byte range out of bounds"),
            });
        }
        match self {
            Self::Memory(bytes) => {
                bytes
                    .get(start..end)
                    .map(Cow::Borrowed)
                    .ok_or(ParseError::Malformed {
                        message: bounded("byte range out of bounds"),
                    })
            }
            Self::SpillFile { file, .. } => {
                let mut buffer = vec![0_u8; end.saturating_sub(start)];
                read_exact_at(file, &mut buffer, start)?;
                Ok(Cow::Owned(buffer))
            }
        }
    }

    fn byte(&self, pos: usize) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        match self {
            Self::Memory(bytes) => bytes.get(pos).copied(),
            Self::SpillFile { file, .. } => {
                let mut byte = [0_u8; 1];
                read_exact_at(file, &mut byte, pos).ok()?;
                Some(byte[0])
            }
        }
    }

    fn starts_with(&self, pos: usize, expected: &[u8]) -> bool {
        let Some(end) = pos.checked_add(expected.len()) else {
            return false;
        };
        self.slice(pos, end)
            .is_ok_and(|bytes| bytes.as_ref() == expected)
    }

    fn find_bytes(&self, needle: &[u8], start: usize, end: usize) -> Option<usize> {
        if needle.is_empty() || start > end || end > self.len() {
            return None;
        }
        match self {
            Self::Memory(_) => {
                let bytes = self.slice(start, end).ok()?;
                find_bytes(bytes.as_ref(), needle, 0)
                    .and_then(|relative| start.checked_add(relative))
            }
            Self::SpillFile { file, .. } => find_bytes_in_spill_file(file, needle, start, end),
        }
    }

    fn stream_source(
        &self,
        start: usize,
        end: usize,
    ) -> std::result::Result<(Arc<[u8]>, StreamRange), ParseError> {
        match self {
            Self::Memory(bytes) => Ok((
                Arc::clone(bytes),
                StreamRange {
                    start: u64::try_from(start).map_err(|_| ParseError::ArithmeticOverflow {
                        context: "stream start",
                    })?,
                    end: u64::try_from(end).map_err(|_| ParseError::ArithmeticOverflow {
                        context: "stream end",
                    })?,
                },
            )),
            Self::SpillFile { .. } => {
                let bytes = self.slice(start, end)?.into_owned();
                let end =
                    u64::try_from(bytes.len()).map_err(|_| ParseError::ArithmeticOverflow {
                        context: "stream end",
                    })?;
                Ok((Arc::from(bytes), StreamRange { start: 0, end }))
            }
        }
    }
}

fn read_exact_at(
    file: &SpillFileHandle,
    buffer: &mut [u8],
    offset: usize,
) -> std::result::Result<(), ParseError> {
    let mut file = file.try_clone().map_err(|_| ParseError::Malformed {
        message: bounded("failed to clone spill file handle"),
    })?;
    file.seek(SeekFrom::Start(u64::try_from(offset).map_err(|_| {
        ParseError::ArithmeticOverflow {
            context: "spill file offset",
        }
    })?))
    .map_err(|_| ParseError::Malformed {
        message: bounded("failed to seek spill file"),
    })?;
    file.read_exact(buffer).map_err(|_| ParseError::Malformed {
        message: bounded("failed to read spill file"),
    })
}

fn find_bytes_in_spill_file(
    file: &SpillFileHandle,
    needle: &[u8],
    start: usize,
    end: usize,
) -> Option<usize> {
    let overlap = needle.len().saturating_sub(1);
    let mut pos = start;
    let mut carried = Vec::new();
    while pos < end {
        let read_len = end.saturating_sub(pos).min(SPILL_SEARCH_CHUNK_BYTES);
        let mut chunk = vec![0_u8; read_len];
        read_exact_at(file, &mut chunk, pos).ok()?;
        let search_base = pos.saturating_sub(carried.len());
        let carried_len = carried.len();
        carried.extend_from_slice(&chunk);
        if let Some(relative) = find_bytes(&carried, needle, 0) {
            return search_base.checked_add(relative);
        }
        if carried.len() > overlap {
            let keep_start = carried.len().saturating_sub(overlap);
            carried = carried.get(keep_start..)?.to_vec();
        }
        pos = pos.checked_add(read_len)?;
        if read_len == 0 || carried_len == carried.len() && chunk.is_empty() {
            break;
        }
    }
    None
}

impl DecoderRegistry {
    /// Creates the default pure-Rust decoder registry.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            decoders: BTreeMap::new(),
        };
        let flate: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(FlateDecoder);
        let ascii_hex: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(AsciiHexDecoder);
        let ascii85: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(Ascii85Decoder);
        let run_length: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(RunLengthDecoder);
        let lzw: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(LzwDecoder);
        let crypt: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(CryptDecoder);
        let metadata_mode: Arc<dyn StreamDecoder + Send + Sync> = Arc::new(MetadataModeDecoder);
        registry.register_many(["FlateDecode", "Fl"], &flate);
        registry.register_many(["ASCIIHexDecode", "AHx"], &ascii_hex);
        registry.register_many(["ASCII85Decode", "A85"], &ascii85);
        registry.register_many(["RunLengthDecode", "RL"], &run_length);
        registry.register_many(["LZWDecode", "LZW"], &lzw);
        registry.register_many(["Crypt"], &crypt);
        registry.register_many(
            ["DCTDecode", "JPXDecode", "JBIG2Decode", "CCITTFaxDecode"],
            &metadata_mode,
        );
        registry
    }

    /// Registers a decoder for a PDF filter name.
    pub fn register(&mut self, name: PdfName, decoder: &Arc<dyn StreamDecoder + Send + Sync>) {
        self.decoders.insert(name, Arc::clone(decoder));
    }

    fn register_many<const N: usize>(
        &mut self,
        names: [&'static str; N],
        decoder: &Arc<dyn StreamDecoder + Send + Sync>,
    ) {
        for name in names {
            self.register(PdfName::from_static(name), decoder);
        }
    }

    fn decoder(&self, name: &PdfName) -> Option<&Arc<dyn StreamDecoder + Send + Sync>> {
        self.decoders.get(name)
    }
}

impl Default for DecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DecoderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecoderRegistry")
            .field("decoder_count", &self.decoders.len())
            .finish()
    }
}

/// Structured parameters for one stream filter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecodeParams {
    /// Predictor algorithm, where 1 means no predictor.
    pub predictor: u16,
    /// Number of colour components for predictor decoding.
    pub colors: u16,
    /// Bits per component for predictor decoding.
    pub bits_per_component: u16,
    /// Predictor column count.
    pub columns: u32,
    /// LZW early-change mode.
    pub early_change: u8,
    /// Named crypt filter, when `/Filter /Crypt` carries one.
    pub crypt_filter_name: Option<PdfName>,
}

impl Default for DecodeParams {
    fn default() -> Self {
        Self {
            predictor: 1,
            colors: 1,
            bits_per_component: 8,
            columns: 1,
            early_change: 1,
            crypt_filter_name: None,
        }
    }
}

/// Parser options for password-capable parsing.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ParseOptions<'a> {
    /// Optional redacted password for Standard security handler decryption.
    pub password: Option<&'a PasswordSecret>,
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
                    decrypted: false,
                    handler: _,
                    ..
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

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut IndirectObject> {
        self.0.values_mut()
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

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut CosObject> {
        self.0.values_mut()
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
    /// Structured decode parameters aligned with `filters`.
    #[serde(default)]
    pub decode_params: Vec<DecodeParams>,
    /// Shared source bytes used for lazy stream decoding.
    #[serde(skip, default = "empty_source")]
    pub raw_source: Arc<[u8]>,
    /// Whether the `stream` keyword is followed by CRLF.
    pub stream_keyword_crlf_compliant: bool,
    /// Whether `endstream` is preceded by an EOL marker.
    pub endstream_keyword_eol_compliant: bool,
}

impl StreamObject {
    pub(crate) fn remove_crypt_filters(&mut self) {
        let mut next_filters = Vec::with_capacity(self.filters.len());
        let mut next_params = Vec::with_capacity(self.decode_params.len());
        for (index, filter) in self.filters.iter().enumerate() {
            if filter.matches("Crypt") {
                continue;
            }
            next_filters.push(filter.clone());
            next_params.push(self.decode_params.get(index).cloned().unwrap_or_default());
        }
        self.filters = next_filters;
        self.decode_params = next_params;
    }

    pub(crate) fn raw_bytes(&self) -> std::result::Result<&[u8], ParseError> {
        let raw_start =
            usize::try_from(self.raw_range.start).map_err(|_| ParseError::ArithmeticOverflow {
                context: "stream raw range",
            })?;
        let raw_end =
            usize::try_from(self.raw_range.end).map_err(|_| ParseError::ArithmeticOverflow {
                context: "stream raw range",
            })?;
        self.raw_source
            .get(raw_start..raw_end)
            .ok_or(ParseError::Malformed {
                message: bounded("stream raw range out of bounds"),
            })
    }

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
        self.decoded_bytes_with_registry(limits, default_decoder_registry())
            .map(|decoded| decoded.bytes)
    }

    fn decoded_bytes_with_registry(
        &self,
        limits: &ResourceLimits,
        registry: &DecoderRegistry,
    ) -> std::result::Result<DecodedStream, ParseError> {
        let mut current = self.raw_bytes()?.to_vec();
        let mut facts = Vec::new();
        for (index, filter) in self.filters.iter().enumerate() {
            let params = self.decode_params.get(index).cloned().unwrap_or_default();
            let decoder = registry
                .decoder(filter)
                .ok_or(ParseError::UnsupportedFilter {
                    filter: BoundedText::unchecked(String::from_utf8_lossy(filter.as_bytes())),
                })?;
            let input_len = checked_u64_len(current.len(), "stream filter input length")?;
            let output = decoder.decode(&current, &params, limits)?;
            let output_len = checked_u64_len(output.bytes.len(), "stream filter output length")?;
            enforce_decoded_len(output_len, limits.max_stream_decode_bytes)?;
            let filter = filter_identifier(filter)?;
            facts.push(if output.metadata_mode {
                StreamFact::FilterMetadataMode {
                    filter,
                    bytes: output_len,
                }
            } else {
                StreamFact::FilterDecoded {
                    filter,
                    input_bytes: input_len,
                    output_bytes: output_len,
                }
            });
            current = output.bytes;
        }
        let decoded_len = checked_u64_len(current.len(), "decoded stream length")?;
        enforce_decoded_len(decoded_len, limits.max_stream_decode_bytes)?;
        Ok(DecodedStream {
            bytes: current,
            facts,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DecodedStream {
    bytes: Vec<u8>,
    facts: Vec<StreamFact>,
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

struct ByteParser<'a> {
    source: SourceStorage,
    limits: ResourceLimits,
    decoder_registry: Option<DecoderRegistry>,
    options: ParseOptions<'a>,
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

impl<'a> ByteParser<'a> {
    fn new(
        source: SourceStorage,
        limits: ResourceLimits,
        decoder_registry: Option<DecoderRegistry>,
        options: ParseOptions<'a>,
    ) -> Self {
        Self {
            source,
            limits,
            decoder_registry,
            options,
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
        self.parse_top_level_objects(&mut objects, &mut trailers)?;

        let encrypted_catalog = trailers
            .iter()
            .rev()
            .find_map(|trailer| object_ref_from_dictionary(&trailer.dictionary, "Root"));

        if encryption_reference(&trailers).is_some() {
            let fact = encryption_fact(&objects, &trailers);
            #[cfg(feature = "decrypt")]
            if let Err(error) = encryption::classify_encryption(&objects, &trailers, &self.limits)
                && !error.is_encrypted_status()
            {
                return Err(error.into_parse_error().into());
            }
            #[cfg(feature = "decrypt")]
            if let Some(password) = self.options.password {
                match encryption::decrypt_document(
                    &mut objects,
                    &mut trailers,
                    &self.limits,
                    password,
                ) {
                    Ok(summary) => {
                        self.push_fact(summary.into_fact(true));
                    }
                    Err(error) if error.is_encrypted_status() => {
                        self.warnings.push(ValidationWarning::General {
                            message: BoundedText::unchecked(error.safe_message()),
                        });
                        self.push_fact(error.into_fact(fact));
                        return Ok(ParsedDocument {
                            version,
                            catalog: encrypted_catalog,
                            objects,
                            trailers,
                            parse_facts: self.parse_facts,
                            warnings: self.warnings,
                        });
                    }
                    Err(error) => return Err(error.into_parse_error().into()),
                }
            } else if self.options.password.is_none() {
                self.push_fact(fact);
                return Ok(ParsedDocument {
                    version,
                    catalog: encrypted_catalog,
                    objects,
                    trailers,
                    parse_facts: self.parse_facts,
                    warnings: self.warnings,
                });
            }

            #[cfg(not(feature = "decrypt"))]
            {
                self.push_fact(fact);
                return Ok(ParsedDocument {
                    version,
                    catalog: encrypted_catalog,
                    objects,
                    trailers,
                    parse_facts: self.parse_facts,
                    warnings: self.warnings,
                });
            }
        }

        self.materialize_stream_backed_structures(&mut objects, &mut trailers)?;
        let catalog = trailers
            .iter()
            .rev()
            .find_map(|trailer| object_ref_from_dictionary(&trailer.dictionary, "Root"));

        Ok(ParsedDocument {
            version,
            catalog,
            objects,
            trailers,
            parse_facts: self.parse_facts,
            warnings: self.warnings,
        })
    }

    fn parse_top_level_objects(
        &mut self,
        objects: &mut ObjectStore,
        trailers: &mut Vec<Trailer>,
    ) -> Result<()> {
        while self.pos < self.source.len() {
            self.skip_ws_and_comments();
            if self.starts_with(EOF_MARKER) {
                self.parse_post_eof_fact()?;
                return Ok(());
            }
            if self.starts_with(b"startxref") {
                self.skip_line();
                continue;
            }
            if self.starts_with(b"xref") {
                self.parse_xref_and_trailer(trailers)?;
                continue;
            }
            if self.starts_with(b"trailer") {
                self.consume_bytes(b"trailer")?;
                self.skip_ws_and_comments();
                let offset = self.offset()?;
                let dictionary = self.parse_dictionary(0)?;
                self.push_xref_chain_facts(None, offset, &dictionary)?;
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
        Ok(())
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
                self.push_xref_chain_facts(Some(key), offset, &stream.dictionary)?;
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
                let decoded = self.decode_stream(key, &stream)?;
                let decoded_len = checked_u64_len(decoded.bytes.len(), "decoded stream length")?;
                self.push_fact(ParseFact::Stream {
                    object: key,
                    fact: StreamFact::Decoded { bytes: decoded_len },
                });
                let mut parsed_objects = self.parse_object_stream(key, &stream, &decoded.bytes)?;
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
        &mut self,
        stream_key: ObjectKey,
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
        let decoded = self.decode_stream(stream_key, stream)?.bytes;
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

    fn decode_stream(
        &mut self,
        key: ObjectKey,
        stream: &StreamObject,
    ) -> std::result::Result<DecodedStream, ParseError> {
        let decoded = if let Some(registry) = &self.decoder_registry {
            stream.decoded_bytes_with_registry(&self.limits, registry)?
        } else {
            stream.decoded_bytes_with_registry(&self.limits, default_decoder_registry())?
        };
        for fact in &decoded.facts {
            self.push_fact(ParseFact::Stream {
                object: key,
                fact: fact.clone(),
            });
        }
        Ok(decoded)
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
        let mut parser = ByteParser::new(
            SourceStorage::Memory(Arc::from(decoded.to_vec())),
            self.limits.clone(),
            None,
            ParseOptions::default(),
        );
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
        let Some(header_pos) = self.source.find_bytes(HEADER_MARKER, 0, self.source.len()) else {
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
        let text = std::str::from_utf8(token.as_ref()).map_err(|_| ParseError::Malformed {
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
            declared_end.and_then(|offset| endstream_after_optional_eol(&self.source, offset));
        let (data_end, endstream_pos) =
            if let (Some(data_end), Some(keyword_pos)) = (declared_end, declared_keyword) {
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
                    .map_or(self.source.len(), |end| end.min(self.source.len()));
                let keyword_pos = self
                    .source
                    .find_bytes(ENDSTREAM_MARKER, data_start, scan_end)
                    .ok_or(ParseError::Malformed {
                        message: bounded("missing endstream"),
                    })?;
                (
                    trim_eol_before(&self.source, data_start, keyword_pos),
                    keyword_pos,
                )
            };
        let endstream_keyword_eol_compliant = has_eol_before(&self.source, endstream_pos);
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

        let decode_params = stream_decode_params(&dictionary, filters.len());
        let (raw_source, raw_range) = self.source.stream_source(data_start, data_end)?;
        Ok(StreamObject {
            dictionary,
            raw_range,
            declared_length,
            discovered_length,
            filters,
            decode_params,
            raw_source,
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
            if self.pos >= self.source.len() || self.starts_with(b"trailer") {
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
                    || !line_had_eol(&self.source, line_start)
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
            if self.pos >= self.source.len() {
                return Ok(());
            }
            if self.starts_with(b"trailer") {
                self.consume_bytes(b"trailer")?;
                self.skip_ws_and_comments();
                let offset = self.offset()?;
                let dictionary = self.parse_dictionary(0)?;
                self.push_xref_chain_facts(None, offset, &dictionary)?;
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
        let remaining =
            self.source
                .len()
                .saturating_sub(self.pos)
                .saturating_sub(count_trailing_ws(
                    self.slice(self.pos, self.source.len())?.as_ref(),
                ));
        if remaining > 0 {
            self.push_fact(ParseFact::PostEofData {
                bytes: u64::try_from(remaining).map_err(|_| ParseError::ArithmeticOverflow {
                    context: "post eof bytes",
                })?,
            });
        }
        Ok(())
    }

    fn push_xref_chain_facts(
        &mut self,
        object: Option<ObjectKey>,
        offset: u64,
        dictionary: &Dictionary,
    ) -> std::result::Result<(), ParseError> {
        let section = ObjectLocation {
            object,
            offset: Some(offset),
            path: None,
        };
        if let Some(prev) = optional_non_negative_offset(dictionary, "Prev")? {
            self.push_fact(ParseFact::Xref {
                section: section.clone(),
                fact: XrefFact::PrevChain { offset: prev },
            });
        }
        if let Some(hybrid) = optional_non_negative_offset(dictionary, "XRefStm")? {
            self.push_fact(ParseFact::Xref {
                section,
                fact: XrefFact::HybridReference { offset: hybrid },
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
        let slice = self.source.slice(self.pos, end).ok()?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.pos = end;
        std::str::from_utf8(slice.as_ref())
            .ok()?
            .parse::<u64>()
            .ok()
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
        let text = std::str::from_utf8(token.as_ref()).map_err(|_| ParseError::Malformed {
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
        self.source.starts_with(self.pos, expected)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.source.byte(self.pos)
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos = self.pos.saturating_add(1);
        Some(byte)
    }

    fn slice(&self, start: usize, end: usize) -> std::result::Result<Cow<'_, [u8]>, ParseError> {
        self.source.slice(start, end)
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

fn has_eol_before(source: &SourceStorage, pos: usize) -> bool {
    matches!(
        pos.checked_sub(1).and_then(|index| source.byte(index)),
        Some(b'\n' | b'\r')
    )
}

fn line_had_eol(source: &SourceStorage, line_start: usize) -> bool {
    let Some(relative) = source.find_bytes(b"\n", line_start, source.len()) else {
        return source.find_bytes(b"\r", line_start, source.len()).is_some();
    };
    relative >= line_start
}

fn endstream_after_optional_eol(source: &SourceStorage, offset: usize) -> Option<usize> {
    if source.starts_with(offset, ENDSTREAM_MARKER) {
        return Some(offset);
    }
    if source.starts_with(offset, b"\r\nendstream") {
        return offset.checked_add(2);
    }
    if source.starts_with(offset, b"\nendstream") || source.starts_with(offset, b"\rendstream") {
        return offset.checked_add(1);
    }
    None
}

fn trim_eol_before(source: &SourceStorage, data_start: usize, keyword_pos: usize) -> usize {
    if keyword_pos >= data_start.saturating_add(2)
        && source
            .slice(keyword_pos.saturating_sub(2), keyword_pos)
            .is_ok_and(|bytes| bytes.as_ref() == b"\r\n")
    {
        return keyword_pos.saturating_sub(2);
    }
    if keyword_pos > data_start
        && matches!(
            source.byte(keyword_pos.saturating_sub(1)),
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

fn optional_non_negative_offset(
    dictionary: &Dictionary,
    key: &'static str,
) -> std::result::Result<Option<u64>, ParseError> {
    let Some(value) = integer_from_dictionary(dictionary, key) else {
        return Ok(None);
    };
    u64::try_from(value)
        .map(Some)
        .map_err(|_| ParseError::Malformed {
            message: BoundedText::unchecked(format!("invalid xref offset dictionary key {key}")),
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
        Some(CosObject::Name(_)) if is_identity_crypt_filter(dictionary, 0) => Vec::new(),
        Some(CosObject::Name(name)) => vec![name.clone()],
        Some(CosObject::Array(values)) => values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| match value {
                CosObject::Name(name) if is_identity_crypt_filter(dictionary, index) => None,
                CosObject::Name(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn is_identity_crypt_filter(dictionary: &Dictionary, filter_index: usize) -> bool {
    let Some(filter) = dictionary.get("Filter") else {
        return false;
    };
    let filter_is_crypt = match filter {
        CosObject::Name(name) => name.matches("Crypt"),
        CosObject::Array(filters) => matches!(
            filters.get(filter_index),
            Some(CosObject::Name(name)) if name.matches("Crypt")
        ),
        _ => false,
    };
    if !filter_is_crypt {
        return false;
    }
    match dictionary.get("DecodeParms") {
        Some(CosObject::Dictionary(params)) => matches!(
            params.get("Name"),
            Some(CosObject::Name(name)) if name.matches("Identity")
        ),
        Some(CosObject::Array(params)) => matches!(
            params.get(filter_index),
            Some(CosObject::Dictionary(params)) if matches!(
                params.get("Name"),
                Some(CosObject::Name(name)) if name.matches("Identity")
            )
        ),
        _ => false,
    }
}

fn stream_decode_params(dictionary: &Dictionary, filter_count: usize) -> Vec<DecodeParams> {
    match dictionary.get("DecodeParms") {
        Some(CosObject::Dictionary(params)) => vec![decode_params_from_dictionary(params)],
        Some(CosObject::Array(values)) => values
            .iter()
            .take(filter_count)
            .map(|value| match value {
                CosObject::Dictionary(params) => decode_params_from_dictionary(params),
                _ => DecodeParams::default(),
            })
            .collect(),
        _ => vec![DecodeParams::default(); filter_count],
    }
}

fn decode_params_from_dictionary(dictionary: &Dictionary) -> DecodeParams {
    DecodeParams {
        predictor: integer_from_dictionary(dictionary, "Predictor")
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(1),
        colors: integer_from_dictionary(dictionary, "Colors")
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(1),
        bits_per_component: integer_from_dictionary(dictionary, "BitsPerComponent")
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(8),
        columns: integer_from_dictionary(dictionary, "Columns")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(1),
        early_change: integer_from_dictionary(dictionary, "EarlyChange")
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(1),
        crypt_filter_name: match dictionary.get("Name") {
            Some(CosObject::Name(name)) => Some(name.clone()),
            _ => None,
        },
    }
}

fn checked_u64_len(len: usize, context: &'static str) -> std::result::Result<u64, ParseError> {
    u64::try_from(len).map_err(|_| ParseError::ArithmeticOverflow { context })
}

fn enforce_decoded_len(len: u64, max_decode_bytes: u64) -> std::result::Result<(), ParseError> {
    if len > max_decode_bytes {
        return Err(ParseError::LimitExceeded {
            limit: "max_stream_decode_bytes",
        });
    }
    Ok(())
}

fn filter_identifier(filter: &PdfName) -> std::result::Result<Identifier, ParseError> {
    Identifier::new(String::from_utf8_lossy(filter.as_bytes()).into_owned()).map_err(|_| {
        ParseError::Malformed {
            message: bounded("stream filter name is not a valid identifier"),
        }
    })
}

#[derive(Debug)]
struct FlateDecoder;

impl StreamDecoder for FlateDecoder {
    fn decode(
        &self,
        input: &[u8],
        params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        let decoded = decode_flate_limited(input, limits.max_stream_decode_bytes)?;
        Ok(DecoderOutput {
            bytes: apply_predictor(decoded, params, limits.max_stream_decode_bytes)?,
            metadata_mode: false,
        })
    }
}

#[derive(Debug)]
struct AsciiHexDecoder;

impl StreamDecoder for AsciiHexDecoder {
    fn decode(
        &self,
        input: &[u8],
        _params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        let mut output = Vec::new();
        let mut high: Option<u8> = None;
        for byte in input {
            if is_ws(*byte) {
                continue;
            }
            if *byte == b'>' {
                break;
            }
            let Some(nibble) = decode_hex_digit(*byte) else {
                return Err(ParseError::StreamDecode {
                    message: bounded("invalid ASCIIHex digit"),
                });
            };
            if let Some(previous) = high.take() {
                push_limited_byte(
                    &mut output,
                    previous.saturating_mul(16).saturating_add(nibble),
                    limits.max_stream_decode_bytes,
                )?;
            } else {
                high = Some(nibble);
            }
        }
        if let Some(previous) = high {
            push_limited_byte(
                &mut output,
                previous.saturating_mul(16),
                limits.max_stream_decode_bytes,
            )?;
        }
        Ok(DecoderOutput {
            bytes: output,
            metadata_mode: false,
        })
    }
}

#[derive(Debug)]
struct Ascii85Decoder;

impl StreamDecoder for Ascii85Decoder {
    fn decode(
        &self,
        input: &[u8],
        _params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        let mut output = Vec::new();
        let mut group = Vec::with_capacity(5);
        let mut iter = input.iter().copied().peekable();
        while let Some(byte) = iter.next() {
            if is_ws(byte) {
                continue;
            }
            if byte == b'~' && iter.peek() == Some(&b'>') {
                break;
            }
            if byte == b'z' {
                if !group.is_empty() {
                    return Err(ParseError::StreamDecode {
                        message: bounded("ASCII85 z inside a partial group"),
                    });
                }
                extend_limited(&mut output, &[0, 0, 0, 0], limits.max_stream_decode_bytes)?;
                continue;
            }
            if !(b'!'..=b'u').contains(&byte) {
                return Err(ParseError::StreamDecode {
                    message: bounded("invalid ASCII85 digit"),
                });
            }
            group.push(byte.saturating_sub(b'!'));
            if group.len() == 5 {
                append_ascii85_group(&mut output, &group, 4, limits.max_stream_decode_bytes)?;
                group.clear();
            }
        }
        if !group.is_empty() {
            let output_bytes = group.len().saturating_sub(1);
            while group.len() < 5 {
                group.push(84);
            }
            append_ascii85_group(
                &mut output,
                &group,
                output_bytes,
                limits.max_stream_decode_bytes,
            )?;
        }
        Ok(DecoderOutput {
            bytes: output,
            metadata_mode: false,
        })
    }
}

#[derive(Debug)]
struct RunLengthDecoder;

impl StreamDecoder for RunLengthDecoder {
    fn decode(
        &self,
        input: &[u8],
        _params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        let mut output = Vec::new();
        let mut pos = 0_usize;
        while let Some(length) = input.get(pos).copied() {
            pos = pos.saturating_add(1);
            match length {
                128 => break,
                0..=127 => {
                    let count = usize::from(length).saturating_add(1);
                    let end = pos
                        .checked_add(count)
                        .ok_or(ParseError::ArithmeticOverflow {
                            context: "RunLength literal",
                        })?;
                    let literal = input.get(pos..end).ok_or(ParseError::StreamDecode {
                        message: bounded("RunLength literal exceeds input"),
                    })?;
                    extend_limited(&mut output, literal, limits.max_stream_decode_bytes)?;
                    pos = end;
                }
                _ => {
                    let Some(value) = input.get(pos).copied() else {
                        return Err(ParseError::StreamDecode {
                            message: bounded("RunLength repeat missing byte"),
                        });
                    };
                    pos = pos.saturating_add(1);
                    let count = 257_usize.saturating_sub(usize::from(length));
                    for _ in 0..count {
                        push_limited_byte(&mut output, value, limits.max_stream_decode_bytes)?;
                    }
                }
            }
        }
        Ok(DecoderOutput {
            bytes: output,
            metadata_mode: false,
        })
    }
}

#[derive(Debug)]
struct LzwDecoder;

impl StreamDecoder for LzwDecoder {
    fn decode(
        &self,
        input: &[u8],
        params: &DecodeParams,
        limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        let decoded = decode_lzw(input, params.early_change, limits.max_stream_decode_bytes)?;
        Ok(DecoderOutput {
            bytes: apply_predictor(decoded, params, limits.max_stream_decode_bytes)?,
            metadata_mode: false,
        })
    }
}

#[derive(Debug)]
struct CryptDecoder;

impl StreamDecoder for CryptDecoder {
    fn decode(
        &self,
        input: &[u8],
        params: &DecodeParams,
        _limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        if params
            .crypt_filter_name
            .as_ref()
            .is_none_or(|name| name.matches("Identity"))
        {
            return Ok(DecoderOutput {
                bytes: input.to_vec(),
                metadata_mode: false,
            });
        }
        Err(ParseError::UnsupportedFilter {
            filter: BoundedText::unchecked("Crypt"),
        })
    }
}

#[derive(Debug)]
struct MetadataModeDecoder;

impl StreamDecoder for MetadataModeDecoder {
    fn decode(
        &self,
        input: &[u8],
        _params: &DecodeParams,
        _limits: &ResourceLimits,
    ) -> std::result::Result<DecoderOutput, ParseError> {
        Ok(DecoderOutput {
            bytes: input.to_vec(),
            metadata_mode: true,
        })
    }
}

fn append_ascii85_group(
    output: &mut Vec<u8>,
    group: &[u8],
    output_bytes: usize,
    max_decode_bytes: u64,
) -> std::result::Result<(), ParseError> {
    let mut value = 0_u32;
    for digit in group {
        value = value
            .checked_mul(85)
            .and_then(|current| current.checked_add(u32::from(*digit)))
            .ok_or(ParseError::StreamDecode {
                message: bounded("ASCII85 group overflows"),
            })?;
    }
    let bytes = value.to_be_bytes();
    let Some(slice) = bytes.get(..output_bytes) else {
        return Err(ParseError::StreamDecode {
            message: bounded("invalid ASCII85 group length"),
        });
    };
    extend_limited(output, slice, max_decode_bytes)
}

fn push_limited_byte(
    output: &mut Vec<u8>,
    byte: u8,
    max_decode_bytes: u64,
) -> std::result::Result<(), ParseError> {
    let next_len = checked_u64_len(output.len(), "decoded stream length")?
        .checked_add(1)
        .ok_or(ParseError::ArithmeticOverflow {
            context: "decoded stream length",
        })?;
    enforce_decoded_len(next_len, max_decode_bytes)?;
    output.push(byte);
    Ok(())
}

fn extend_limited(
    output: &mut Vec<u8>,
    bytes: &[u8],
    max_decode_bytes: u64,
) -> std::result::Result<(), ParseError> {
    let next_len = checked_u64_len(output.len(), "decoded stream length")?
        .checked_add(checked_u64_len(bytes.len(), "decoded stream length")?)
        .ok_or(ParseError::ArithmeticOverflow {
            context: "decoded stream length",
        })?;
    enforce_decoded_len(next_len, max_decode_bytes)?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn apply_predictor(
    bytes: Vec<u8>,
    params: &DecodeParams,
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    match params.predictor {
        1 => Ok(bytes),
        2 => apply_tiff_predictor(bytes, params, max_decode_bytes),
        10..=15 => apply_png_predictor(&bytes, params, max_decode_bytes),
        _ => Err(ParseError::StreamDecode {
            message: bounded("unsupported predictor"),
        }),
    }
}

fn predictor_geometry(params: &DecodeParams) -> std::result::Result<(usize, usize), ParseError> {
    if params.colors == 0 || params.bits_per_component == 0 || params.columns == 0 {
        return Err(ParseError::StreamDecode {
            message: bounded("invalid predictor geometry"),
        });
    }
    let bits_per_row = u64::from(params.colors)
        .checked_mul(u64::from(params.bits_per_component))
        .and_then(|bits| bits.checked_mul(u64::from(params.columns)))
        .ok_or(ParseError::ArithmeticOverflow {
            context: "predictor row size",
        })?;
    let row_bytes = bits_per_row
        .checked_add(7)
        .ok_or(ParseError::ArithmeticOverflow {
            context: "predictor row size",
        })?
        / 8;
    let bytes_per_pixel_bits = u64::from(params.colors)
        .checked_mul(u64::from(params.bits_per_component))
        .ok_or(ParseError::ArithmeticOverflow {
            context: "predictor pixel size",
        })?;
    let bytes_per_pixel =
        bytes_per_pixel_bits
            .checked_add(7)
            .ok_or(ParseError::ArithmeticOverflow {
                context: "predictor pixel size",
            })?
            / 8;
    Ok((
        usize::try_from(row_bytes).map_err(|_| ParseError::LimitExceeded {
            limit: "max_stream_decode_bytes",
        })?,
        usize::try_from(bytes_per_pixel.max(1)).map_err(|_| ParseError::LimitExceeded {
            limit: "max_stream_decode_bytes",
        })?,
    ))
}

fn apply_tiff_predictor(
    mut bytes: Vec<u8>,
    params: &DecodeParams,
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    enforce_decoded_len(
        checked_u64_len(bytes.len(), "predictor output length")?,
        max_decode_bytes,
    )?;
    let (row_bytes, bytes_per_pixel) = predictor_geometry(params)?;
    if row_bytes == 0 || !bytes.len().is_multiple_of(row_bytes) {
        return Err(ParseError::StreamDecode {
            message: bounded("TIFF predictor row length mismatch"),
        });
    }
    for row in bytes.chunks_mut(row_bytes) {
        for index in bytes_per_pixel..row.len() {
            let left = row
                .get(index.saturating_sub(bytes_per_pixel))
                .copied()
                .ok_or(ParseError::StreamDecode {
                    message: bounded("TIFF predictor left byte missing"),
                })?;
            let Some(byte) = row.get_mut(index) else {
                return Err(ParseError::StreamDecode {
                    message: bounded("TIFF predictor byte missing"),
                });
            };
            *byte = byte.wrapping_add(left);
        }
    }
    Ok(bytes)
}

fn apply_png_predictor(
    bytes: &[u8],
    params: &DecodeParams,
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    let (row_bytes, bytes_per_pixel) = predictor_geometry(params)?;
    let encoded_row = row_bytes
        .checked_add(1)
        .ok_or(ParseError::ArithmeticOverflow {
            context: "PNG predictor row size",
        })?;
    if encoded_row == 0 || !bytes.len().is_multiple_of(encoded_row) {
        return Err(ParseError::StreamDecode {
            message: bounded("PNG predictor row length mismatch"),
        });
    }
    let row_count = bytes.len() / encoded_row;
    let output_capacity =
        row_count
            .checked_mul(row_bytes)
            .ok_or(ParseError::ArithmeticOverflow {
                context: "PNG predictor output length",
            })?;
    enforce_decoded_len(
        checked_u64_len(output_capacity, "PNG predictor output length")?,
        max_decode_bytes,
    )?;
    let mut output = vec![0_u8; output_capacity];
    for row_index in 0..row_count {
        let encoded_start =
            row_index
                .checked_mul(encoded_row)
                .ok_or(ParseError::ArithmeticOverflow {
                    context: "PNG predictor row offset",
                })?;
        let filter = *bytes.get(encoded_start).ok_or(ParseError::StreamDecode {
            message: bounded("PNG predictor filter byte missing"),
        })?;
        let encoded = bytes
            .get(encoded_start.saturating_add(1)..encoded_start.saturating_add(encoded_row))
            .ok_or(ParseError::StreamDecode {
                message: bounded("PNG predictor row missing"),
            })?;
        let output_start =
            row_index
                .checked_mul(row_bytes)
                .ok_or(ParseError::ArithmeticOverflow {
                    context: "PNG predictor output row offset",
                })?;
        for index in 0..row_bytes {
            let raw = *encoded.get(index).ok_or(ParseError::StreamDecode {
                message: bounded("PNG predictor source byte missing"),
            })?;
            let left = if index >= bytes_per_pixel {
                output
                    .get(output_start + index - bytes_per_pixel)
                    .copied()
                    .ok_or(ParseError::StreamDecode {
                        message: bounded("PNG predictor left byte missing"),
                    })?
            } else {
                0
            };
            let up = if row_index > 0 {
                output
                    .get(output_start + index - row_bytes)
                    .copied()
                    .ok_or(ParseError::StreamDecode {
                        message: bounded("PNG predictor upper byte missing"),
                    })?
            } else {
                0
            };
            let up_left = if row_index > 0 && index >= bytes_per_pixel {
                output
                    .get(output_start + index - row_bytes - bytes_per_pixel)
                    .copied()
                    .ok_or(ParseError::StreamDecode {
                        message: bounded("PNG predictor upper-left byte missing"),
                    })?
            } else {
                0
            };
            let value = png_predictor_value(filter, raw, left, up, up_left)?;
            let Some(target) = output.get_mut(output_start + index) else {
                return Err(ParseError::StreamDecode {
                    message: bounded("PNG predictor target byte missing"),
                });
            };
            *target = value;
        }
    }
    Ok(output)
}

fn png_predictor_value(
    filter: u8,
    raw: u8,
    left: u8,
    up: u8,
    up_left: u8,
) -> std::result::Result<u8, ParseError> {
    match filter {
        0 => Ok(raw),
        1 => Ok(raw.wrapping_add(left)),
        2 => Ok(raw.wrapping_add(up)),
        3 => {
            let average =
                u8::try_from(u16::midpoint(u16::from(left), u16::from(up))).map_err(|_| {
                    ParseError::StreamDecode {
                        message: bounded("PNG predictor average byte out of range"),
                    }
                })?;
            Ok(raw.wrapping_add(average))
        }
        4 => Ok(raw.wrapping_add(paeth_predictor(left, up, up_left))),
        _ => Err(ParseError::StreamDecode {
            message: bounded("invalid PNG predictor filter"),
        }),
    }
}

fn paeth_predictor(left: u8, up: u8, up_left: u8) -> u8 {
    let left = i16::from(left);
    let up = i16::from(up);
    let up_left = i16::from(up_left);
    let estimate = left + up - up_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let up_left_distance = (estimate - up_left).abs();
    if left_distance <= up_distance && left_distance <= up_left_distance {
        u8::try_from(left).unwrap_or(0)
    } else if up_distance <= up_left_distance {
        u8::try_from(up).unwrap_or(0)
    } else {
        u8::try_from(up_left).unwrap_or(0)
    }
}

fn decode_lzw(
    input: &[u8],
    early_change: u8,
    max_decode_bytes: u64,
) -> std::result::Result<Vec<u8>, ParseError> {
    let mut reader = MsbBitReader::new(input);
    let mut dictionary = initial_lzw_dictionary();
    let mut code_bits = 9_u8;
    let mut next_code = 258_u16;
    let mut previous: Option<Vec<u8>> = None;
    let mut output = Vec::new();
    while let Some(code) = reader.read_bits(code_bits)? {
        match code {
            256 => {
                dictionary = initial_lzw_dictionary();
                code_bits = 9;
                next_code = 258;
                previous = None;
            }
            257 => break,
            _ => {
                let entry = if let Some(value) = dictionary.get(usize::from(code)).cloned() {
                    value
                } else if code == next_code {
                    let mut value = previous.clone().ok_or(ParseError::StreamDecode {
                        message: bounded("LZW missing previous entry"),
                    })?;
                    let first = *value.first().ok_or(ParseError::StreamDecode {
                        message: bounded("LZW empty previous entry"),
                    })?;
                    value.push(first);
                    value
                } else {
                    return Err(ParseError::StreamDecode {
                        message: bounded("invalid LZW code"),
                    });
                };
                extend_limited(&mut output, &entry, max_decode_bytes)?;
                if let Some(previous_entry) = previous {
                    let mut new_entry = previous_entry;
                    let first = *entry.first().ok_or(ParseError::StreamDecode {
                        message: bounded("LZW empty entry"),
                    })?;
                    new_entry.push(first);
                    if dictionary.len() < 4096 {
                        dictionary.push(new_entry);
                        next_code = next_code.saturating_add(1);
                        let threshold =
                            (1_u16 << code_bits).saturating_sub(u16::from(early_change.min(1)));
                        if next_code >= threshold && code_bits < 12 {
                            code_bits = code_bits.saturating_add(1);
                        }
                    }
                }
                previous = Some(entry);
            }
        }
    }
    Ok(output)
}

fn initial_lzw_dictionary() -> Vec<Vec<u8>> {
    let mut dictionary = Vec::with_capacity(258);
    for byte in 0_u8..=255 {
        dictionary.push(vec![byte]);
    }
    dictionary.push(Vec::new());
    dictionary.push(Vec::new());
    dictionary
}

#[derive(Clone, Copy, Debug)]
struct MsbBitReader<'a> {
    input: &'a [u8],
    bit_pos: usize,
}

impl<'a> MsbBitReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, bit_pos: 0 }
    }

    fn read_bits(&mut self, bits: u8) -> std::result::Result<Option<u16>, ParseError> {
        let remaining_bits = self
            .input
            .len()
            .checked_mul(8)
            .and_then(|total| total.checked_sub(self.bit_pos))
            .ok_or(ParseError::ArithmeticOverflow {
                context: "LZW bit position",
            })?;
        if remaining_bits < usize::from(bits) {
            return Ok(None);
        }
        let mut value = 0_u16;
        for _ in 0..bits {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7_usize.saturating_sub(self.bit_pos % 8);
            let byte = self
                .input
                .get(byte_index)
                .copied()
                .ok_or(ParseError::StreamDecode {
                    message: bounded("LZW bit read out of bounds"),
                })?;
            value = value.checked_shl(1).ok_or(ParseError::ArithmeticOverflow {
                context: "LZW code",
            })? | u16::from((byte >> bit_index) & 1);
            self.bit_pos = self.bit_pos.saturating_add(1);
        }
        Ok(Some(value))
    }
}

fn encryption_reference(trailers: &[Trailer]) -> Option<&CosObject> {
    trailers
        .iter()
        .rev()
        .find_map(|trailer| trailer.dictionary.get("Encrypt"))
}

fn encryption_fact(objects: &ObjectStore, trailers: &[Trailer]) -> ParseFact {
    #[cfg(feature = "decrypt")]
    {
        encryption::encryption_summary(objects, trailers).into_fact(false)
    }
    #[cfg(not(feature = "decrypt"))]
    {
        let handler = trailers.iter().rev().find_map(|trailer| {
            let encrypt = trailer.dictionary.get("Encrypt")?;
            match encrypt {
                CosObject::Dictionary(dictionary) => encryption_handler(dictionary),
                CosObject::Reference(key) => objects
                    .get(key)
                    .and_then(|object| object.object.as_dictionary())
                    .and_then(encryption_handler),
                _ => None,
            }
        });
        ParseFact::Encryption {
            encrypted: true,
            handler,
            version: None,
            revision: None,
            algorithm: None,
            decrypted: false,
        }
    }
}

#[cfg(not(feature = "decrypt"))]
fn encryption_handler(dictionary: &Dictionary) -> Option<Identifier> {
    let Some(CosObject::Name(filter)) = dictionary.get("Filter") else {
        return None;
    };
    Identifier::new(String::from_utf8_lossy(filter.as_bytes()).into_owned()).ok()
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
    use std::{error::Error, io::Cursor, num::NonZeroU32};

    use proptest::prelude::*;
    use rstest::rstest;

    use super::{CosObject, ParsedDocument, Parser, StreamObject};
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
    fn test_should_treat_identity_crypt_stream_filter_as_passthrough() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
2 0 obj
<< /Length 3 /Filter /Crypt /DecodeParms << /Name /Identity >> >>
stream
abc
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";

        let document = Parser::default().parse(Cursor::new(bytes))?;
        let object = document
            .objects
            .get(&crate::ObjectKey::new(
                NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
                0,
            ))
            .ok_or_else(|| crate::ParseError::MissingObject {
                message: crate::BoundedText::unchecked("missing stream object"),
            })?;
        let CosObject::Stream(stream) = &object.object else {
            return Err(crate::ParseError::Malformed {
                message: crate::BoundedText::unchecked("missing stream"),
            }
            .into());
        };

        assert_eq!(stream.decoded_bytes(&ResourceLimits::default())?, b"abc");
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
    fn test_should_emit_xref_prev_and_hybrid_reference_facts() -> crate::Result<()> {
        let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
xref
0 2
0000000000 65535 f
0000000009 00000 n
trailer
<< /Size 2 /Root 1 0 R >>
xref
0 1
0000000000 65535 f
trailer
<< /Size 2 /Root 1 0 R /Prev 40 /XRefStm 120 >>
%%EOF
";

        let document = Parser::default().parse(Cursor::new(bytes))?;

        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Xref {
                    fact: crate::XrefFact::PrevChain { offset: 40 },
                    ..
                }
            )
        }));
        assert!(document.parse_facts.iter().any(|fact| {
            matches!(
                fact,
                ParseFact::Xref {
                    fact: crate::XrefFact::HybridReference { offset: 120 },
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
    fn test_should_decode_asciihex_ascii85_runlength_and_lzw_streams() -> crate::Result<()> {
        let cases: [(&str, Vec<u8>, &[u8]); 4] = [
            ("ASCIIHexDecode", b"61 62>".to_vec(), b"ab"),
            ("ASCII85Decode", b"9jqo~>".to_vec(), b"Man"),
            (
                "RunLengthDecode",
                vec![2, b'a', b'b', b'c', 254, b'x', 128],
                b"abcxxx",
            ),
            (
                "LZWDecode",
                pack_lzw_codes(&[(256, 9), (97, 9), (98, 9), (97, 9), (257, 9)]),
                b"aba",
            ),
        ];
        for (filter, encoded, expected) in cases {
            let document =
                Parser::default().parse(Cursor::new(single_stream_pdf(filter, "", &encoded)))?;
            let stream = parsed_stream(&document)?;

            assert_eq!(stream.decoded_bytes(&ResourceLimits::default())?, expected);
        }
        Ok(())
    }

    #[test]
    fn test_should_apply_flate_png_predictor() -> Result<(), Box<dyn Error>> {
        use std::io::Write;

        use flate2::{Compression, write::ZlibEncoder};

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[1, b'a', 1, 1])?;
        let compressed = encoder.finish()?;
        let document = Parser::default().parse(Cursor::new(single_stream_pdf(
            "FlateDecode",
            "/DecodeParms << /Predictor 12 /Columns 3 >>",
            &compressed,
        )))?;
        let stream = parsed_stream(&document)?;

        assert_eq!(stream.decoded_bytes(&ResourceLimits::default())?, b"abc");
        Ok(())
    }

    #[test]
    fn test_should_emit_per_filter_decode_facts_for_object_stream() -> crate::Result<()> {
        let object_stream = b"1 0 << /Type /Catalog >>";
        let encoded = hex_bytes(object_stream);
        let mut bytes = br"%PDF-1.7
2 0 obj
<< /Type /ObjStm /N 1 /First 4 /Filter /ASCIIHexDecode /Length "
            .to_vec();
        bytes.extend(encoded.len().to_string().as_bytes());
        bytes.extend(
            br" >>
stream
",
        );
        bytes.extend(encoded);
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
                    fact: StreamFact::FilterDecoded {
                        filter,
                        output_bytes: 24,
                        ..
                    },
                    ..
                } if filter.as_str() == "ASCIIHexDecode"
            )
        }));
        Ok(())
    }

    #[test]
    fn test_should_preserve_image_filter_bytes_in_metadata_mode() -> crate::Result<()> {
        let document =
            Parser::default().parse(Cursor::new(single_stream_pdf("DCTDecode", "", b"image")))?;
        let stream = parsed_stream(&document)?;

        assert_eq!(stream.decoded_bytes(&ResourceLimits::default())?, b"image");
        Ok(())
    }

    #[test]
    fn test_should_parse_with_spill_file_source_storage_above_threshold() -> crate::Result<()> {
        let limits = ResourceLimits {
            memory_source_threshold_bytes: 0,
            ..ResourceLimits::default()
        };
        let document = Parser::new(limits.clone()).parse(Cursor::new(single_stream_pdf(
            "ASCIIHexDecode",
            "",
            b"61 62>",
        )))?;
        let stream = parsed_stream(&document)?;

        assert_eq!(stream.decoded_bytes(&limits)?, b"ab");
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

    fn single_stream_pdf(filter: &str, params: &str, encoded: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "%PDF-1.7\n1 0 obj\n<< /Length {} /Filter /{filter} {params} >>\nstream\n",
            encoded.len()
        )
        .into_bytes();
        bytes.extend(encoded);
        bytes.extend(b"\nendstream\nendobj\n%%EOF\n");
        bytes
    }

    fn parsed_stream(document: &ParsedDocument) -> crate::Result<&StreamObject> {
        let object =
            document
                .objects
                .values()
                .next()
                .ok_or_else(|| crate::ParseError::MissingObject {
                    message: crate::BoundedText::unchecked("missing stream object"),
                })?;
        let CosObject::Stream(stream) = &object.object else {
            return Err(crate::ParseError::Malformed {
                message: crate::BoundedText::unchecked("missing stream"),
            }
            .into());
        };
        Ok(stream)
    }

    fn pack_lzw_codes(codes: &[(u16, u8)]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut current = 0_u8;
        let mut used = 0_u8;
        for (code, bits) in codes {
            for bit in (0..*bits).rev() {
                current <<= 1;
                current |= u8::try_from((code >> bit) & 1).unwrap_or(0);
                used = used.saturating_add(1);
                if used == 8 {
                    output.push(current);
                    current = 0;
                    used = 0;
                }
            }
        }
        if used != 0 {
            current <<= 8_u8.saturating_sub(used);
            output.push(current);
        }
        output
    }

    fn hex_bytes(bytes: &[u8]) -> Vec<u8> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = Vec::with_capacity(bytes.len().saturating_mul(2).saturating_add(1));
        for byte in bytes {
            output.push(HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'));
            output.push(HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0'));
        }
        output.push(b'>');
        output
    }
}
