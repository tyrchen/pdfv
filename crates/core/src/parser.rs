//! Tolerant byte-level PDF parser used by the validator.

#[cfg(feature = "decrypt")]
mod encryption;
mod xref;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, btree_map},
    fmt,
    io::{Read, Seek, SeekFrom, Write},
    num::NonZeroU32,
    ops::RangeInclusive,
    sync::{Arc, OnceLock},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use self::xref::{ClassicXref, XrefEntries, XrefEntry};
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

        ByteParser::new(storage, self.limits.clone(), self.decoder_registry.clone())
            .parse_document(options)
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
    /// Parsed trailers, ending with the primary chain in revision order when resolved.
    pub trailers: Vec<Trailer>,
    /// Parser facts retained for validation.
    pub parse_facts: Vec<ParseFact>,
    /// Recoverable parser warnings.
    pub warnings: Vec<ValidationWarning>,
    #[serde(default)]
    pub(crate) file_syntax: FileSyntax,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) signed_revision_ends: Vec<RangeInclusive<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_trailer_start: Option<usize>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    decrypted: bool,
}

/// Fixed-size syntax observations used by conformance rules.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FileSyntax {
    pub(crate) header_comment: [u8; 4],
    pub(crate) xref_keyword_eol_compliant: bool,
    pub(crate) xref_subsection_space_separated: bool,
}

impl Default for FileSyntax {
    fn default() -> Self {
        Self {
            header_comment: [0; 4],
            xref_keyword_eol_compliant: true,
            xref_subsection_space_separated: true,
        }
    }
}

impl ParsedDocument {
    pub(crate) fn active_trailers(&self) -> &[Trailer] {
        self.trailers
            .get(self.active_trailer_start.unwrap_or(0)..)
            .unwrap_or_default()
    }

    pub(crate) fn has_encryption(&self) -> bool {
        encryption_reference(self.active_trailers()).is_some()
    }

    /// Returns true when the document declares encryption and still needs decryption.
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.has_encryption()
            && !self.decrypted
            && !self.parse_facts.iter().any(|fact| {
                matches!(
                    fact,
                    ParseFact::Encryption {
                        encrypted: true,
                        decrypted: true,
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

    #[cfg(feature = "decrypt")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) spacing_compliant: Option<bool>,
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
    /// Looks up an entry by a parsed PDF name.
    pub(crate) fn get_name(&self, key: &PdfName) -> Option<&CosObject> {
        self.0.get(key)
    }

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
        self.entries()
    }

    pub(crate) fn entries(&self) -> btree_map::Iter<'_, PdfName, CosObject> {
        self.0.iter()
    }

    #[cfg(feature = "decrypt")]
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
///
/// JSON deserialization accepts legacy byte arrays. When the parser records
/// the lexical form, serialization includes the bytes and syntax together.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(from = "PdfStringWire", into = "PdfStringWire")]
pub struct PdfString {
    bytes: Vec<u8>,
    syntax: PdfStringSyntax,
    pub(crate) file_range: Option<StreamRange>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PdfStringSyntax {
    Unknown,
    Literal,
    Hex { digits: usize },
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum PdfStringWire {
    Bytes(Vec<u8>),
    Parsed {
        bytes: Vec<u8>,
        syntax: PdfStringSyntax,
        #[serde(default, rename = "fileRange", skip_serializing_if = "Option::is_none")]
        file_range: Option<StreamRange>,
    },
}

impl PartialEq for PdfString {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for PdfString {}

impl From<PdfStringWire> for PdfString {
    fn from(value: PdfStringWire) -> Self {
        match value {
            PdfStringWire::Bytes(bytes) => Self {
                bytes,
                syntax: PdfStringSyntax::Unknown,
                file_range: None,
            },
            PdfStringWire::Parsed {
                bytes,
                syntax,
                file_range,
            } => Self {
                bytes,
                syntax,
                file_range,
            },
        }
    }
}

impl From<PdfString> for PdfStringWire {
    fn from(value: PdfString) -> Self {
        match value.syntax {
            PdfStringSyntax::Unknown => Self::Bytes(value.bytes),
            syntax => Self::Parsed {
                bytes: value.bytes,
                syntax,
                file_range: value.file_range,
            },
        }
    }
}

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
        Ok(Self {
            bytes,
            syntax: PdfStringSyntax::Unknown,
            file_range: None,
        })
    }

    /// Returns raw string bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn syntax(&self) -> PdfStringSyntax {
        self.syntax
    }

    fn with_syntax(mut self, syntax: PdfStringSyntax) -> Self {
        self.syntax = syntax;
        self
    }
}

impl TryFrom<Vec<u8>> for PdfString {
    type Error = ConfigError;

    fn try_from(value: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            bytes: value,
            syntax: PdfStringSyntax::Unknown,
            file_range: None,
        })
    }
}

impl From<PdfString> for Vec<u8> {
    fn from(value: PdfString) -> Self {
        value.bytes
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
    /// Whether the `stream` keyword is followed by LF or CRLF.
    pub stream_keyword_crlf_compliant: bool,
    /// Whether `endstream` is preceded by an EOL marker.
    pub endstream_keyword_eol_compliant: bool,
}

impl StreamObject {
    #[cfg(feature = "decrypt")]
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

    pub(crate) fn fully_decoded_bytes(
        &self,
        limits: &ResourceLimits,
        work_left: Option<&mut u64>,
    ) -> std::result::Result<Vec<u8>, ParseError> {
        self.decode_stream_data(limits, default_decoder_registry(), true, work_left)
            .map(|decoded| decoded.bytes)
    }

    fn decoded_bytes_with_registry(
        &self,
        limits: &ResourceLimits,
        registry: &DecoderRegistry,
    ) -> std::result::Result<DecodedStream, ParseError> {
        self.decode_stream_data(limits, registry, false, None)
    }

    fn decode_stream_data(
        &self,
        limits: &ResourceLimits,
        registry: &DecoderRegistry,
        require_payload: bool,
        mut work_left: Option<&mut u64>,
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
            let output = if let Some(available) = work_left.as_deref_mut() {
                decode_under_budget(decoder.as_ref(), &current, &params, limits, available)?
            } else {
                decoder.decode(&current, &params, limits)?
            };
            let output_len = checked_u64_len(output.bytes.len(), "stream filter output length")?;
            enforce_decoded_len(output_len, limits.max_stream_decode_bytes)?;
            if output.metadata_mode && require_payload {
                return Err(ParseError::UnsupportedFilter {
                    filter: BoundedText::unchecked(String::from_utf8_lossy(filter.as_bytes())),
                });
            }
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

fn decode_under_budget(
    decoder: &dyn StreamDecoder,
    input: &[u8],
    params: &DecodeParams,
    limits: &ResourceLimits,
    work_left: &mut u64,
) -> std::result::Result<DecoderOutput, ParseError> {
    if *work_left == 0 {
        return Err(ParseError::LimitExceeded {
            limit: "embedded PDF filter output work",
        });
    }
    let mut stage_limits = limits.clone();
    stage_limits.max_stream_decode_bytes = limits.max_stream_decode_bytes.min(*work_left);
    // Charge the reservation if the decoder fails without returning its partial output.
    *work_left = work_left.saturating_sub(stage_limits.max_stream_decode_bytes);
    let output = decoder.decode(input, params, &stage_limits)?;
    let used = checked_u64_len(output.bytes.len(), "stream filter output length")?;
    enforce_decoded_len(used, stage_limits.max_stream_decode_bytes)?;
    let unused = stage_limits.max_stream_decode_bytes.saturating_sub(used);
    *work_left = work_left.saturating_add(unused);
    Ok(output)
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

struct ByteParser {
    source: SourceStorage,
    limits: ResourceLimits,
    decoder_registry: Option<DecoderRegistry>,
    pos: usize,
    end: usize,
    parse_facts: Vec<ParseFact>,
    warnings: Vec<ValidationWarning>,
    file_syntax: FileSyntax,
    startxref: Option<u64>,
    startxref_marker: Option<usize>,
    revision_probe_budget: usize,
    classic_xrefs: BTreeMap<u64, ClassicXref>,
    active_trailer_start: Option<usize>,
    file_offsets: bool,
    signed_revision_ends: Vec<RangeInclusive<u64>>,
    decrypted: bool,
}

#[derive(Clone, Copy, Debug)]
enum NumberToken {
    Integer(i64),
    Real(f64),
}

enum RevisionContinuation {
    LinearizedBody,
    IncrementalUpdate,
}

#[derive(Debug)]
struct XrefStreamSummary {
    decoded_bytes: usize,
    entries: u64,
    compressed_entries: u64,
    objects: XrefEntries,
}

pub(crate) fn parse_text_operand(
    bytes: &[u8],
    limits: &ResourceLimits,
    array: bool,
) -> std::result::Result<Vec<PdfString>, ParseError> {
    let mut parser = ByteParser::new(
        SourceStorage::Memory(Arc::from(bytes)),
        limits.clone(),
        None,
    );
    match (array, parser.parse_object(0)?) {
        (false, CosObject::String(string)) => Ok(vec![string]),
        (true, CosObject::Array(items)) => {
            let mut strings = Vec::new();
            for item in items {
                match item {
                    CosObject::String(string) => strings.push(string),
                    CosObject::Integer(_) | CosObject::Real(_) => {}
                    _ => {
                        return Err(ParseError::Malformed {
                            message: bounded("invalid TJ array member"),
                        });
                    }
                }
            }
            Ok(strings)
        }
        _ => Err(ParseError::Malformed {
            message: bounded("invalid text-show operand"),
        }),
    }
}

pub(crate) fn parse_content_dictionary(
    bytes: &[u8],
    limits: &ResourceLimits,
) -> std::result::Result<Dictionary, ParseError> {
    let mut parser = ByteParser::new(
        SourceStorage::Memory(Arc::from(bytes)),
        limits.clone(),
        None,
    );
    parser.parse_dictionary(0)
}

pub(crate) fn parse_content_array(
    bytes: &[u8],
    limits: &ResourceLimits,
) -> std::result::Result<Vec<CosObject>, ParseError> {
    ByteParser::new(
        SourceStorage::Memory(Arc::from(bytes)),
        limits.clone(),
        None,
    )
    .parse_array(0)
}

impl ByteParser {
    fn new(
        source: SourceStorage,
        limits: ResourceLimits,
        decoder_registry: Option<DecoderRegistry>,
    ) -> Self {
        let end = source.len();
        let revision_probe_budget = end.saturating_mul(2);
        Self {
            source,
            limits,
            decoder_registry,
            pos: 0,
            end,
            parse_facts: Vec::new(),
            warnings: Vec::new(),
            file_syntax: FileSyntax::default(),
            startxref: None,
            startxref_marker: None,
            revision_probe_budget,
            classic_xrefs: BTreeMap::new(),
            active_trailer_start: None,
            file_offsets: false,
            signed_revision_ends: Vec::new(),
            decrypted: false,
        }
    }

    fn parse_document(mut self, options: ParseOptions<'_>) -> Result<ParsedDocument> {
        #[cfg(not(feature = "decrypt"))]
        let _ = options;

        let (header_offset, version) = self.parse_header()?;
        self.push_fact(ParseFact::Header {
            offset: header_offset,
            version,
            had_leading_bytes: header_offset != 0,
        });

        let mut physical = BTreeMap::new();
        let mut trailers = Vec::new();
        self.file_offsets = true;
        self.parse_top_level_objects(&mut physical, &mut trailers)?;
        let active_xref = self.resolve_xref(&physical, &mut trailers)?;
        let mut objects = xref::select_objects(physical, active_xref.as_ref())?;

        let active_trailers = trailers
            .get(self.active_trailer_start.unwrap_or(0)..)
            .unwrap_or_default();
        let catalog = active_trailers
            .iter()
            .rev()
            .find_map(|trailer| object_ref_from_dictionary(&trailer.dictionary, "Root"));

        if encryption_reference(active_trailers).is_some() {
            let fact = encryption_fact(&objects, active_trailers);
            #[cfg(feature = "decrypt")]
            if let Err(error) =
                encryption::classify_encryption(&objects, active_trailers, &self.limits)
                && !error.is_encrypted_status()
            {
                return Err(error.into_parse_error().into());
            }
            #[cfg(feature = "decrypt")]
            if let Some(password) = options.password {
                match encryption::decrypt_document(
                    &mut objects,
                    active_trailers,
                    &self.limits,
                    password,
                ) {
                    Ok(summary) => {
                        self.decrypted = true;
                        self.push_fact(summary.into_fact(true));
                    }
                    Err(error) if error.is_encrypted_status() => {
                        self.warnings.push(ValidationWarning::General {
                            message: BoundedText::unchecked(error.safe_message()),
                        });
                        self.push_fact(error.into_fact(fact));
                        return Ok(self.finish(version, catalog, objects, trailers));
                    }
                    Err(error) => return Err(error.into_parse_error().into()),
                }
            } else if options.password.is_none() {
                self.push_fact(fact);
                return Ok(self.finish(version, catalog, objects, trailers));
            }

            #[cfg(not(feature = "decrypt"))]
            {
                self.push_fact(fact);
                return Ok(self.finish(version, catalog, objects, trailers));
            }
        }

        self.materialize_object_streams(&mut objects, active_xref.as_ref())?;
        xref::verify_compressed_entries(&objects, active_xref.as_ref())?;

        Ok(self.finish(version, catalog, objects, trailers))
    }

    fn finish(
        self,
        version: PdfVersion,
        catalog: Option<ObjectKey>,
        objects: ObjectStore,
        trailers: Vec<Trailer>,
    ) -> ParsedDocument {
        ParsedDocument {
            version,
            catalog,
            objects,
            trailers,
            parse_facts: self.parse_facts,
            warnings: self.warnings,
            file_syntax: self.file_syntax,
            signed_revision_ends: self.signed_revision_ends,
            active_trailer_start: self.active_trailer_start,
            decrypted: self.decrypted,
        }
    }

    fn parse_top_level_objects(
        &mut self,
        objects: &mut BTreeMap<u64, IndirectObject>,
        trailers: &mut Vec<Trailer>,
    ) -> Result<()> {
        let mut revision_sections = BTreeSet::new();
        while self.pos < self.end {
            self.skip_ws_and_comments();
            if self.starts_with(EOF_MARKER) {
                if let Some(offset) = self.startxref
                    && (self.classic_xrefs.contains_key(&offset)
                        || objects.get(&offset).and_then(xref::xref_stream).is_some())
                {
                    revision_sections.insert(offset);
                }
                if let Some(continuation) =
                    self.revision_continuation(objects, &revision_sections)?
                {
                    if matches!(continuation, RevisionContinuation::IncrementalUpdate) {
                        self.record_signed_revision_end()?;
                    }
                    self.consume_bytes(EOF_MARKER)?;
                    continue;
                }
                self.record_signed_revision_end()?;
                self.parse_post_eof_fact()?;
                return Ok(());
            }
            if self.starts_with(b"startxref") {
                let start = self.pos;
                self.consume_bytes(b"startxref")?;
                self.startxref = self.parse_unsigned::<u64>().ok().flatten();
                self.startxref_marker = self.startxref.map(|_| start);
                if self.startxref.is_none() {
                    self.pos = start;
                }
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
                    objects.insert(object.offset, object);
                }
                None => {
                    self.pos = before.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    fn record_signed_revision_end(&mut self) -> Result<()> {
        if u64::try_from(self.signed_revision_ends.len()).unwrap_or(u64::MAX)
            >= self.limits.max_objects
        {
            return Err(ParseError::LimitExceeded {
                limit: "max_objects",
            }
            .into());
        }
        let marker_end = self.pos.saturating_add(EOF_MARKER.len());
        let to_offset = |position| {
            u64::try_from(position).map_err(|_| ParseError::ArithmeticOverflow {
                context: "signed revision end",
            })
        };
        let line_end = offset_after_eol(&self.source, marker_end).unwrap_or(marker_end);
        let minimum = if line_end == self.end {
            line_end
        } else {
            marker_end
        };
        self.signed_revision_ends
            .push(to_offset(minimum)?..=to_offset(line_end)?);
        Ok(())
    }

    fn revision_continuation(
        &mut self,
        objects: &BTreeMap<u64, IndirectObject>,
        revision_sections: &BTreeSet<u64>,
    ) -> std::result::Result<Option<RevisionContinuation>, ParseError> {
        if self.startxref.is_none() {
            return Ok(None);
        }
        let eof_end = self.pos.saturating_add(EOF_MARKER.len());
        let mut cursor = eof_end;
        let mut examined = 0_u64;
        while let Some(marker) = self.source.find_bytes(b"startxref", cursor, self.end) {
            cursor = marker.saturating_add(b"startxref".len());
            examined = examined.saturating_add(1);
            if examined > self.limits.max_objects {
                return Err(ParseError::LimitExceeded {
                    limit: "revision_markers",
                });
            }
            let Some((section, eof)) = self.revision_marker(marker) else {
                continue;
            };
            if !revision_sections.is_empty() && (eof_end..marker).contains(&section) {
                if self
                    .revision_trailer(section, marker)?
                    .and_then(|dictionary| {
                        optional_non_negative_offset(&dictionary, "Prev")
                            .ok()
                            .flatten()
                    })
                    .is_some_and(|previous| revision_sections.contains(&previous))
                {
                    return Ok(Some(RevisionContinuation::IncrementalUpdate));
                }
            } else if section < self.pos
                && self.closes_linearized_body(objects, section, marker, eof)?
            {
                return Ok(Some(RevisionContinuation::LinearizedBody));
            }
        }
        Ok(None)
    }

    fn revision_marker(&self, marker: usize) -> Option<(usize, usize)> {
        if !line_starts_at(&self.source, marker) {
            return None;
        }
        let source = self.source.clone();
        let cursor = marker.checked_add(b"startxref".len())?;
        let mut probe = Self::new(source, self.limits.clone(), None);
        probe.pos = cursor;
        // The integer and EOF must occupy their own lines.
        probe.skip_xref_spaces();
        probe.pos = offset_after_eol(&probe.source, probe.pos)?;
        probe.skip_xref_spaces();
        if !probe.peek_byte()?.is_ascii_digit() {
            return None;
        }
        let section = probe.parse_unsigned::<usize>().ok()??;
        probe.skip_xref_spaces();
        probe.pos = offset_after_eol(&probe.source, probe.pos)?;
        let eof = probe.pos;
        if !probe.starts_with(EOF_MARKER) {
            return None;
        }
        probe.pos = probe.pos.checked_add(EOF_MARKER.len())?;
        probe.skip_xref_spaces();
        (probe.pos == probe.source.len() || offset_after_eol(&probe.source, probe.pos).is_some())
            .then_some((section, eof))
    }

    fn closes_linearized_body(
        &mut self,
        objects: &BTreeMap<u64, IndirectObject>,
        section: usize,
        marker: usize,
        eof: usize,
    ) -> std::result::Result<bool, ParseError> {
        let Some(dictionary) = objects
            .first_key_value()
            .map(|(_, object)| object)
            .and_then(|object| object.object.as_dictionary())
        else {
            return Ok(false);
        };
        if !matches!(dictionary.get("Linearized"), Some(CosObject::Integer(1))) {
            return Ok(false);
        }
        let Some(body_end) =
            integer_from_dictionary(dictionary, "L").and_then(|value| usize::try_from(value).ok())
        else {
            return Ok(false);
        };
        let eof_end = eof.saturating_add(EOF_MARKER.len());
        if !(eof_end..=self.end).contains(&body_end) {
            return Ok(false);
        }
        self.charge_revision_probe(body_end.saturating_sub(eof_end))?;
        if !self
            .source
            .slice(eof_end, body_end)?
            .iter()
            .copied()
            .all(is_ws)
        {
            return Ok(false);
        }
        let Some(early_marker) = self.startxref_marker.filter(|marker| section < *marker) else {
            return Ok(false);
        };
        let Some(main_section) = self
            .revision_trailer(section, early_marker)?
            .and_then(|dictionary| {
                optional_non_negative_offset(&dictionary, "Prev")
                    .ok()
                    .flatten()
            })
            .and_then(|offset| usize::try_from(offset).ok())
            .filter(|offset| (self.pos..marker).contains(offset))
        else {
            return Ok(false);
        };
        Ok(self.revision_trailer(main_section, marker)?.is_some())
    }

    fn revision_trailer(
        &mut self,
        section: usize,
        marker: usize,
    ) -> std::result::Result<Option<Dictionary>, ParseError> {
        self.charge_revision_probe(marker.saturating_sub(section))?;
        let mut probe = Self::new(self.source.clone(), self.limits.clone(), None);
        probe.pos = section;
        probe.end = marker;
        let dictionary = if probe.starts_with(b"xref") {
            let Some(dictionary) = probe.probe_classic_trailer().ok().flatten() else {
                return Ok(None);
            };
            dictionary
        } else {
            let Ok(Some(object)) = probe.parse_indirect_object() else {
                return Ok(None);
            };
            let CosObject::Stream(stream) = object.object else {
                return Ok(None);
            };
            if !matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("XRef"))
            {
                return Ok(None);
            }
            stream.dictionary
        };
        probe.skip_ws_and_comments();
        Ok((probe.pos == probe.end).then_some(dictionary))
    }

    fn probe_classic_trailer(&mut self) -> std::result::Result<Option<Dictionary>, ParseError> {
        self.consume_bytes(b"xref")?;
        self.skip_xref_spaces();
        if offset_after_eol(&self.source, self.pos).is_none() {
            return Ok(None);
        }
        let (well_formed, _, has_subsection) = self.read_classic_xref_entries()?;
        if !well_formed || !has_subsection || !self.starts_with(b"trailer") {
            return Ok(None);
        }
        self.consume_bytes(b"trailer")?;
        self.skip_ws_and_comments();
        self.parse_dictionary(0).map(Some)
    }

    fn charge_revision_probe(&mut self, bytes: usize) -> std::result::Result<(), ParseError> {
        self.revision_probe_budget =
            self.revision_probe_budget
                .checked_sub(bytes)
                .ok_or(ParseError::LimitExceeded {
                    limit: "revision_probe_bytes",
                })?;
        Ok(())
    }

    fn materialize_object_streams(
        &mut self,
        objects: &mut ObjectStore,
        xref: Option<&XrefEntries>,
    ) -> std::result::Result<(), ParseError> {
        let mut expanded = ObjectStore::default();
        for container in objects.values() {
            let CosObject::Stream(stream) = &container.object else {
                continue;
            };
            if !matches!(stream.dictionary.get("Type"), Some(CosObject::Name(name)) if name.matches("ObjStm"))
            {
                continue;
            }
            let decoded = self.decode_stream(container.key, stream)?;
            self.push_fact(ParseFact::Stream {
                object: container.key,
                fact: StreamFact::Decoded {
                    bytes: checked_u64_len(decoded.bytes.len(), "decoded stream length")?,
                },
            });
            let parsed = self.parse_object_stream(container.key, stream, &decoded.bytes)?;
            for (index, object) in parsed.into_iter().enumerate() {
                if let Some(xref) = xref {
                    let entry = XrefEntry::Compressed {
                        stream: container.key,
                        index: checked_u64_len(index, "object stream index")?,
                    };
                    if xref.get(&object.key.number) != Some(&entry) {
                        continue;
                    }
                }
                if objects.get(&object.key).is_some() || expanded.get(&object.key).is_some() {
                    continue;
                }
                let count = objects.len().saturating_add(expanded.len());
                if checked_u64_len(count, "object count")? >= self.limits.max_objects {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_objects",
                    });
                }
                expanded.insert(object);
            }
            self.push_fact(ParseFact::Xref {
                section: ObjectLocation {
                    object: Some(container.key),
                    offset: Some(container.offset),
                    path: None,
                },
                fact: XrefFact::ObjectStreamParsed,
            });
        }
        objects.0.append(&mut expanded.0);
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
        let mut objects = BTreeMap::new();
        for (first_object, count) in indexes {
            for index in 0..count {
                let entry_type = if widths[0] == 0 {
                    1
                } else {
                    read_be_uint(&decoded, &mut pos, widths[0])?
                };
                let field_two = read_be_uint(&decoded, &mut pos, widths[1])?;
                let field_three = read_be_uint(&decoded, &mut pos, widths[2])?;
                if entry_type == 2 {
                    compressed_entries = compressed_entries.checked_add(1).ok_or(
                        ParseError::ArithmeticOverflow {
                            context: "compressed xref entries",
                        },
                    )?;
                }
                let number = first_object.checked_add(index).and_then(xref_object_number);
                if let Some(number) = number {
                    let entry = match entry_type {
                        1 => {
                            u16::try_from(field_three).map_or(XrefEntry::Malformed, |generation| {
                                XrefEntry::Uncompressed {
                                    offset: field_two,
                                    generation,
                                }
                            })
                        }
                        2 => xref_object_number(field_two).map_or(XrefEntry::Malformed, |number| {
                            XrefEntry::Compressed {
                                stream: ObjectKey::new(number, 0),
                                index: field_three,
                            }
                        }),
                        _ => XrefEntry::Unavailable,
                    };
                    if objects.insert(number, entry).is_some() {
                        return Err(ParseError::Malformed {
                            message: bounded("duplicate object number in xref stream"),
                        });
                    }
                }
            }
        }
        Ok(XrefStreamSummary {
            decoded_bytes: decoded.len(),
            entries: total_entries,
            compressed_entries,
            objects,
        })
    }

    fn decode_stream(
        &mut self,
        key: ObjectKey,
        stream: &StreamObject,
    ) -> std::result::Result<DecodedStream, ParseError> {
        let registry = self
            .decoder_registry
            .as_ref()
            .unwrap_or_else(|| default_decoder_registry());
        let decoded = stream.decode_stream_data(&self.limits, registry, true, None)?;
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
                spacing_compliant: None,
            });
        }
        Ok(objects)
    }

    fn parse_header(&mut self) -> std::result::Result<(u64, PdfVersion), ParseError> {
        let Some(header_pos) = self.source.find_bytes(HEADER_MARKER, 0, self.end) else {
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
        self.file_syntax.header_comment = header_comment_bytes(&self.source, self.pos);
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
        let after_number = self.pos;
        self.skip_required_ws()?;
        let first_separator = self.pos.saturating_sub(after_number) == 1;
        let Some(generation) = self.parse_unsigned_u16()? else {
            self.pos = start;
            return Ok(None);
        };
        let after_generation = self.pos;
        self.skip_required_ws()?;
        let second_separator = self.pos.saturating_sub(after_generation) == 1;
        if !self.starts_with(b"obj") {
            self.pos = start;
            return Ok(None);
        }
        self.consume_bytes(b"obj")?;
        let starts_line = start
            .checked_sub(1)
            .and_then(|index| self.source.byte(index));
        let header_spacing = first_separator
            && second_separator
            && matches!(starts_line, Some(b'\r' | b'\n'))
            && matches!(self.peek_byte(), Some(b'\r' | b'\n'));
        let Some(number) = NonZeroU32::new(number) else {
            return Err(ParseError::Malformed {
                message: bounded("object number must be non-zero"),
            });
        };
        let key = ObjectKey { number, generation };
        let object_start = u64::try_from(start).map_err(|_| ParseError::ArithmeticOverflow {
            context: "object offset",
        })?;

        let parsed = self.parse_spanned_object(0, true)?;
        let object = match parsed {
            CosObject::Dictionary(dictionary) if self.peek_stream_marker() => {
                CosObject::Stream(self.parse_stream(key, dictionary)?)
            }
            other => other,
        };
        self.skip_ws_and_comments();
        let end_spacing = if self.starts_with(ENDOBJ_MARKER) {
            let before_end = self
                .pos
                .checked_sub(1)
                .and_then(|index| self.source.byte(index));
            self.consume_bytes(ENDOBJ_MARKER)?;
            matches!(before_end, Some(b'\r' | b'\n'))
                && matches!(self.peek_byte(), Some(b'\r' | b'\n'))
        } else {
            false
        };
        Ok(Some(IndirectObject {
            key,
            offset: object_start,
            object,
            spacing_compliant: Some(header_spacing && end_spacing),
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
            let value =
                self.parse_spanned_object(depth.saturating_add(1), key.matches("Contents"))?;
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

    fn parse_spanned_object(
        &mut self,
        depth: u32,
        record_range: bool,
    ) -> std::result::Result<CosObject, ParseError> {
        if !self.file_offsets || !record_range {
            return self.parse_object(depth);
        }
        self.skip_ws_and_comments();
        let start = self.offset()?;
        let mut value = self.parse_object(depth)?;
        if let CosObject::String(text) = &mut value {
            text.file_range = Some(StreamRange {
                start,
                end: self.offset()?,
            });
        }
        Ok(value)
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
                    if let Some(decoded) = self.parse_literal_string_escape()? {
                        bytes.push(decoded);
                    }
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
                        return PdfString::new(bytes, &self.limits)
                            .map(|value| value.with_syntax(PdfStringSyntax::Literal));
                    }
                    bytes.push(byte);
                }
                b'\r' => {
                    self.skip_optional_lf();
                    bytes.push(b'\n');
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

    fn parse_literal_string_escape(&mut self) -> std::result::Result<Option<u8>, ParseError> {
        let escaped = self.next_byte().ok_or(ParseError::Malformed {
            message: bounded("truncated string escape"),
        })?;
        let decoded = match escaped {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'b' => 0x08,
            b'f' => 0x0c,
            b'\r' => {
                self.skip_optional_lf();
                return Ok(None);
            }
            b'\n' => return Ok(None),
            b'0'..=b'7' => {
                let mut value = escaped.saturating_sub(b'0');
                for _ in 0..2 {
                    let Some(digit @ b'0'..=b'7') = self.peek_byte() else {
                        break;
                    };
                    self.pos = self.pos.saturating_add(1);
                    value = value
                        .wrapping_mul(8)
                        .wrapping_add(digit.saturating_sub(b'0'));
                }
                value
            }
            other => other,
        };
        Ok(Some(decoded))
    }

    fn skip_optional_lf(&mut self) {
        if self.peek_byte() == Some(b'\n') {
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn parse_hex_string(&mut self) -> std::result::Result<PdfString, ParseError> {
        self.consume_bytes(b"<")?;
        let mut bytes = Vec::new();
        let mut high = None;
        let mut digits = 0_usize;
        loop {
            let byte = self.next_byte().ok_or(ParseError::Malformed {
                message: bounded("unterminated hex string"),
            })?;
            if byte == b'>' {
                break;
            }
            if is_ws(byte) {
                continue;
            }
            let nibble = decode_hex_digit(byte).ok_or(ParseError::Malformed {
                message: bounded("invalid hex string"),
            })?;
            digits = digits.saturating_add(1);
            if let Some(first) = high.take() {
                bytes.push((first << 4) | nibble);
            } else {
                if bytes.len() >= self.limits.max_string_bytes {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_string_bytes",
                    });
                }
                high = Some(nibble);
            }
        }
        if let Some(first) = high {
            bytes.push(first << 4);
        }
        PdfString::new(bytes, &self.limits)
            .map(|value| value.with_syntax(PdfStringSyntax::Hex { digits }))
    }

    fn parse_number_or_reference(&mut self) -> std::result::Result<CosObject, ParseError> {
        match self.parse_number_token()? {
            NumberToken::Integer(value) => {
                let after_number = self.pos;
                if let Some(key) = self.parse_reference_suffix(value)? {
                    return Ok(CosObject::Reference(key));
                }
                self.pos = after_number;
                Ok(CosObject::Integer(value))
            }
            NumberToken::Real(value) => Ok(CosObject::Real(value)),
        }
    }

    fn parse_reference_suffix(
        &mut self,
        number: i64,
    ) -> std::result::Result<Option<ObjectKey>, ParseError> {
        let start = self.pos;
        self.skip_ws_and_comments();
        let generation_start = self.pos;
        if generation_start == start {
            return Ok(None);
        }
        self.skip_ascii_digits();
        let generation_end = self.pos;
        if generation_start == generation_end {
            return Ok(None);
        }
        self.skip_ws_and_comments();
        if self.pos == generation_end
            || !self.starts_with(b"R")
            || self
                .source
                .byte(self.pos.saturating_add(1))
                .is_some_and(|byte| !is_ws(byte) && !is_delimiter(byte))
        {
            return Ok(None);
        }
        let token = self.slice(generation_start, generation_end)?;
        let generation = std::str::from_utf8(token.as_ref())
            .ok()
            .and_then(|digits| digits.parse::<u16>().ok())
            .ok_or(ParseError::Malformed {
                message: bounded("reference generation out of range"),
            })?;
        let number =
            u32::try_from(number)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or(ParseError::Malformed {
                    message: bounded("reference object number out of range"),
                })?;
        self.consume_bytes(b"R")?;
        Ok(Some(ObjectKey { number, generation }))
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
        let stream_keyword_crlf_compliant = self.starts_with(b"\r\n") || self.starts_with(b"\n");
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
                    .map_or(self.end, |end| end.min(self.end));
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
        let keyword_eol_compliant = offset_after_eol(&self.source, self.pos)
            .and_then(|offset| self.source.byte(offset))
            .is_some_and(|byte| byte.is_ascii_digit());
        self.file_syntax.xref_keyword_eol_compliant &= keyword_eol_compliant;
        let entries_start = self.pos;
        let (entries_well_formed, _, has_subsection) = self.read_classic_xref_entries()?;
        let entries_end = self.pos;
        self.push_fact(ParseFact::Xref {
            section: ObjectLocation {
                object: None,
                offset: Some(section_offset),
                path: None,
            },
            fact: if keyword_eol_compliant
                && entries_well_formed
                && self.file_syntax.xref_subsection_space_separated
            {
                XrefFact::EolMarkersComply
            } else {
                XrefFact::MalformedClassic
            },
        });
        if self.starts_with(b"trailer") {
            self.consume_bytes(b"trailer")?;
            self.skip_ws_and_comments();
            let offset = self.offset()?;
            let dictionary = self.parse_dictionary(0)?;
            self.push_xref_chain_facts(None, offset, &dictionary)?;
            if entries_well_formed && has_subsection {
                if checked_u64_len(self.classic_xrefs.len(), "xref sections")?
                    >= self.limits.max_objects
                {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_objects",
                    });
                }
                self.classic_xrefs.insert(
                    section_offset,
                    ClassicXref {
                        entries_range: entries_start..entries_end,
                        trailer_index: trailers.len(),
                    },
                );
            }
            trailers.push(Trailer { dictionary, offset });
        }
        Ok(())
    }

    fn read_classic_xref_entries(
        &mut self,
    ) -> std::result::Result<(bool, XrefEntries, bool), ParseError> {
        let mut compliant = true;
        let mut objects = BTreeMap::new();
        let mut parsed_entries = 0_u64;
        let mut has_subsection = false;
        loop {
            self.skip_ws_and_comments();
            if self.pos >= self.end || self.starts_with(b"trailer") {
                break;
            }
            if self.starts_with(b"startxref") || self.starts_with(EOF_MARKER) {
                break;
            }
            let Some(first_object) = self.parse_unsigned_u32()? else {
                compliant = false;
                self.skip_line();
                continue;
            };
            let separator_start = self.pos;
            self.skip_required_ws()?;
            let space_separated = self.source.byte(separator_start) == Some(b' ')
                && self.pos == separator_start.saturating_add(1)
                && self.peek_byte().is_some_and(|byte| byte.is_ascii_digit());
            self.file_syntax.xref_subsection_space_separated &= space_separated;
            let Some(count) = self.parse_unsigned_u32()? else {
                compliant = false;
                self.skip_line();
                continue;
            };
            has_subsection = true;
            self.skip_line();
            for index in 0..count {
                let next_line = next_line_start(&self.source, self.pos);
                let offset = self.parse_fixed_digits(10);
                self.skip_xref_spaces();
                let generation = self.parse_fixed_digits(5);
                self.skip_xref_spaces();
                let marker = self.next_byte();
                if offset.is_none()
                    || generation.is_none()
                    || !matches!(marker, Some(b'n' | b'f'))
                    || next_line.is_none()
                {
                    compliant = false;
                }
                let number = u64::from(first_object) + u64::from(index);
                if let (Some(offset), Some(generation), Some(marker)) = (offset, generation, marker)
                    && let Ok(generation) = u16::try_from(generation)
                {
                    if let Some(number) = xref_object_number(number) {
                        let entry = match marker {
                            b'n' => XrefEntry::Uncompressed { offset, generation },
                            _ => XrefEntry::Unavailable,
                        };
                        compliant &= objects.insert(number, entry).is_none();
                    } else {
                        compliant &= number == 0 && marker == b'f';
                    }
                } else {
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
                self.pos = next_line.unwrap_or(self.end);
            }
        }
        Ok((compliant, objects, has_subsection))
    }

    fn parse_post_eof_fact(&mut self) -> std::result::Result<(), ParseError> {
        self.consume_bytes(EOF_MARKER)?;
        let line_end = offset_after_eol(&self.source, self.pos).unwrap_or(self.pos);
        let remaining = self.end.saturating_sub(line_end);
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
        self.skip_ascii_digits();
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

    fn skip_ascii_digits(&mut self) {
        while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos = self.pos.saturating_add(1);
        }
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
        self.pos = next_line_start(&self.source, self.pos).unwrap_or(self.end);
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
        expected.len() <= self.end.saturating_sub(self.pos)
            && self.source.starts_with(self.pos, expected)
    }

    fn peek_byte(&self) -> Option<u8> {
        if self.pos >= self.end {
            return None;
        }
        self.source.byte(self.pos)
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos = self.pos.saturating_add(1);
        Some(byte)
    }

    fn slice(&self, start: usize, end: usize) -> std::result::Result<Cow<'_, [u8]>, ParseError> {
        if end > self.end {
            return Err(ParseError::Malformed {
                message: bounded("token exceeds parser input"),
            });
        }
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

fn line_starts_at(source: &SourceStorage, offset: usize) -> bool {
    offset == 0 || matches!(source.byte(offset.saturating_sub(1)), Some(b'\r' | b'\n'))
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

fn offset_after_eol(source: &SourceStorage, pos: usize) -> Option<usize> {
    match source.byte(pos)? {
        b'\r' if source.byte(pos.saturating_add(1)) == Some(b'\n') => pos.checked_add(2),
        b'\r' | b'\n' => pos.checked_add(1),
        _ => None,
    }
}

fn next_line_start(source: &SourceStorage, pos: usize) -> Option<usize> {
    let mut start = pos;
    let mut chunk_len = 64;
    while start < source.len() {
        let end = start.saturating_add(chunk_len).min(source.len());
        let bytes = source.slice(start, end).ok()?;
        if let Some(relative) = bytes.iter().position(|byte| matches!(byte, b'\r' | b'\n')) {
            return offset_after_eol(source, start.checked_add(relative)?);
        }
        start = end;
        chunk_len = SPILL_SEARCH_CHUNK_BYTES;
    }
    None
}

fn header_comment_bytes(source: &SourceStorage, version_end: usize) -> [u8; 4] {
    let Some(comment_start) = next_line_start(source, version_end) else {
        return [0; 4];
    };
    if source.byte(comment_start) != Some(b'%') {
        return [0; 4];
    }
    let mut comment = [0; 4];
    for (index, byte) in comment.iter_mut().enumerate() {
        let offset = comment_start.saturating_add(1).saturating_add(index);
        match source.byte(offset) {
            Some(b'\r' | b'\n') | None => return [0; 4],
            Some(value) => *byte = value,
        }
    }
    comment
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

fn xref_object_number(number: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(u32::try_from(number).ok()?)
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
    use crate::{ObjectKey, ParseError, ParseFact, PdfvError, ResourceLimits, StreamFact};

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

        assert!(document.warnings.iter().any(|warning| matches!(
            warning,
            crate::ValidationWarning::General { message }
                if message.as_str() == "malformed PDF header version recovered as 1.4"
        )));
        assert_eq!(document.version, crate::PdfVersion { major: 1, minor: 4 });
        Ok(())
    }

    #[test]
    fn test_should_round_trip_lexical_string_evidence_and_read_legacy_bytes()
    -> Result<(), Box<dyn Error>> {
        let document = Parser::default().parse(Cursor::new(
            b"%PDF-1.7\n1 0 obj\n<ACE>\nendobj\ntrailer\n<<>>\n%%EOF\n",
        ))?;
        let value = document.objects.values().next().ok_or("missing object")?;
        let CosObject::String(value) = &value.object else {
            return Err("expected PDF string".into());
        };
        let encoded = serde_json::to_string(value)?;
        let decoded: super::PdfString = serde_json::from_str(&encoded)?;
        assert_eq!(decoded.as_bytes(), &[0xac, 0xe0]);
        assert!(matches!(
            decoded.syntax(),
            super::PdfStringSyntax::Hex { digits: 3 }
        ));

        let legacy: super::PdfString = serde_json::from_str("[172,224]")?;
        assert_eq!(legacy, decoded);
        assert!(matches!(legacy.syntax(), super::PdfStringSyntax::Unknown));
        assert_eq!(serde_json::to_string(&legacy)?, "[172,224]");
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

    #[cfg(feature = "flate")]
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

    #[rstest]
    #[case(true, true)]
    #[case(false, true)]
    #[case(true, false)]
    fn test_should_read_only_linked_incremental_revisions(
        #[case] linked: bool,
        #[case] has_entries: bool,
    ) -> crate::Result<()> {
        let mut bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Lang (en) >>\nendobj\n".to_vec();
        let first_xref = bytes.len();
        bytes.extend_from_slice(
            format!(
                "xref\n0 2\n0000000000 65535 f\r\n0000000009 00000 n\r\ntrailer\n<< /Root 1 0 R \
                 /Size 2 >>\nstartxref\n{first_xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        let next_object = bytes.len();
        bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Lang (fr) >>\nendobj\n");
        let second_xref = bytes.len();
        bytes.extend_from_slice(b"xref\n");
        if has_entries {
            bytes.extend_from_slice(format!("1 1\n{next_object:010} 00000 n\r\n").as_bytes());
        }
        let previous = if linked { first_xref } else { 9 };
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Root 1 0 R /Size 2 /Prev {previous} \
                 >>\nstartxref\n{second_xref}\n%%EOF\n"
            )
            .as_bytes(),
        );

        let parsed = Parser::default().parse(Cursor::new(bytes))?;
        let language = parsed
            .catalog
            .and_then(|root| parsed.objects.get(&root))
            .and_then(|object| object.object.as_dictionary())
            .and_then(|dictionary| dictionary.get("Lang"));
        let expected: &[u8] = if linked && has_entries { b"fr" } else { b"en" };
        assert!(matches!(language, Some(CosObject::String(value)) if value.as_bytes() == expected));
        assert_eq!(
            parsed
                .parse_facts
                .iter()
                .any(|fact| matches!(fact, ParseFact::PostEofData { .. })),
            !(linked && has_entries)
        );
        Ok(())
    }

    #[test]
    fn test_should_resolve_classic_revisions_by_offset_and_generation() -> crate::Result<()> {
        let mut bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
        let original = bytes.len();
        bytes.extend_from_slice(b"2 0 obj\n10\nendobj\n3 0 obj\n12\nendobj\n");
        let base_xref = bytes.len();
        let third = original + b"2 0 obj\n10\nendobj\n".len();
        append_classic_xref(
            &mut bytes,
            &format!(
                "0 4\n0000000000 65535 f\r\n0000000009 00000 n\r\n{original:010} 00000 \
                 n\r\n{third:010} 00000 n\r\n"
            ),
            "/Root 1 0 R /Size 4",
        );

        bytes.extend_from_slice(b"2 0 obj\n99\nendobj\n");
        let freed_xref = bytes.len();
        append_classic_xref(
            &mut bytes,
            &format!("2 2\n{original:010} 00000 n\r\n0000000000 00001 f\r\n"),
            &format!("/Size 4 /Prev {base_xref}"),
        );
        let parsed = Parser::default().parse(Cursor::new(&bytes))?;
        assert_eq!(parsed.catalog, test_key(1, 0));
        assert_eq!(integer_object(&parsed, 2, 0), Some(10));
        assert_eq!(integer_object(&parsed, 3, 0), None);
        let mut wrong_offset = bytes.clone();
        append_classic_xref(
            &mut wrong_offset,
            &format!("2 1\n{:010} 00000 n\r\n", original + 1),
            &format!("/Size 4 /Prev {freed_xref}"),
        );
        assert!(matches!(
            Parser::default().parse(Cursor::new(wrong_offset)),
            Err(PdfvError::Parse(ParseError::MissingObject { .. }))
        ));

        let reused = bytes.len();
        bytes.extend_from_slice(b"2 1 obj\n11\nendobj\n");
        let mut alternate = bytes.clone();
        append_classic_xref(
            &mut bytes,
            &format!("2 1\n{reused:010} 00001 n\r\n"),
            &format!("/Size 4 /Prev {freed_xref}"),
        );
        let parsed = Parser::default().parse(Cursor::new(&bytes))?;
        assert_eq!(integer_object(&parsed, 2, 1), Some(11));
        assert_eq!(integer_object(&parsed, 2, 0), None);
        assert_eq!(integer_object(&parsed, 3, 0), None);

        append_classic_xref(
            &mut alternate,
            &format!("2 1\n{reused:010} 00001 n\r\n"),
            &format!("/Size 4 /Prev {base_xref}"),
        );
        let parsed = Parser::default().parse(Cursor::new(alternate))?;
        assert_eq!(integer_object(&parsed, 2, 1), Some(11));
        assert_eq!(integer_object(&parsed, 3, 0), Some(12));
        assert_eq!(parsed.trailers.len(), 3);
        assert_eq!(parsed.active_trailers().len(), 2);
        Ok(())
    }

    #[rstest]
    #[case(2, 0, false, false, Some(42))]
    #[case(2, 0, false, true, Some(43))]
    #[case(2, 1, false, false, None)]
    #[case(0, 1, false, false, None)]
    #[case(2, 0, true, false, Some(42))]
    #[case(2, 0, true, true, None)]
    fn test_should_use_linked_stream_entry_and_member_index(
        #[case] entry_type: u8,
        #[case] member_index: u16,
        #[case] hybrid: bool,
        #[case] classic_update: bool,
        #[case] expected: Option<i64>,
    ) -> Result<(), Box<dyn Error>> {
        let mut bytes =
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n2 0 obj\n10\nendobj\n".to_vec();
        let root_end = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".len();
        let base_xref = bytes.len();
        append_classic_xref(
            &mut bytes,
            &format!("1 2\n0000000009 00000 n\r\n{root_end:010} 00000 n\r\n"),
            "/Root 1 0 R /Size 3",
        );
        let container = u32::try_from(bytes.len())?;
        bytes.extend_from_slice(
            b"4 0 obj\n<< /Type /ObjStm /N 2 /First 8 /Length 13 >>\nstream\n2 0 3 3 42 44\nendstream\nendobj\n",
        );
        let stream_offset = u32::try_from(bytes.len())?;
        let field_two = if entry_type == 2 { 4 } else { 0 };
        let entries = xref_stream_data(&[
            (entry_type, field_two, member_index),
            (1, container, 0),
            (1, stream_offset, 0),
        ]);
        let previous = if hybrid { 9 } else { base_xref };
        bytes.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /XRef /Size 6 /W [1 4 2] /Index [2 1 4 2] /Prev {previous} \
                 /Length {} >>\nstream\n",
                entries.len()
            )
            .as_bytes(),
        );
        bytes.extend(entries);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");

        if hybrid {
            let hybrid_offset = bytes.len();
            append_classic_xref(
                &mut bytes,
                "0 0\n",
                &format!("/Size 6 /Prev {base_xref} /XRefStm {stream_offset}"),
            );
            if classic_update {
                append_classic_xref(
                    &mut bytes,
                    "2 1\n0000000000 00001 f\r\n",
                    &format!("/Size 6 /Prev {hybrid_offset} /XRefStm {stream_offset}"),
                );
            }
        } else {
            bytes.extend_from_slice(format!("startxref\n{stream_offset}\n%%EOF\n").as_bytes());
            if classic_update {
                let object = bytes.len();
                bytes.extend_from_slice(b"2 0 obj\n43\nendobj\n");
                append_classic_xref(
                    &mut bytes,
                    &format!("2 1\n{object:010} 00000 n\r\n"),
                    &format!("/Size 6 /Prev {stream_offset}"),
                );
            }
        }
        let parsed = Parser::default().parse(Cursor::new(bytes));
        if entry_type == 2 && member_index != 0 {
            assert!(matches!(
                parsed,
                Err(PdfvError::Parse(ParseError::MissingObject { .. }))
            ));
            return Ok(());
        }
        let parsed = parsed?;
        assert_eq!(parsed.catalog, test_key(1, 0));
        assert_eq!(integer_object(&parsed, 2, 0), expected);
        assert_eq!(integer_object(&parsed, 3, 0), None);
        if hybrid {
            assert_eq!(parsed.trailers.len(), parsed.active_trailers().len() + 1);
        }
        Ok(())
    }

    #[test]
    fn test_should_follow_linearized_sections_and_ignore_first_page_eof() -> crate::Result<()> {
        let (initial, main) = linearized_pdf(0, 0);
        let (bytes, _) = linearized_pdf(initial.len(), main);
        let parsed = Parser::default().parse(Cursor::new(bytes))?;
        assert_eq!(parsed.catalog, test_key(1, 0));
        assert_eq!(parsed.objects.len(), 2);
        assert!(matches!(
            parsed.trailers.last().and_then(|trailer| trailer.dictionary.get("Info")),
            Some(CosObject::Reference(key)) if Some(*key) == test_key(8, 0)
        ));
        assert_eq!(parsed.signed_revision_ends.len(), 1);
        Ok(())
    }

    fn linearized_pdf(length: usize, main_xref: usize) -> (Vec<u8>, usize) {
        let mut bytes = format!("%PDF-1.7\n8 0 obj\n<< /Linearized 1 /L {length:010} >>\nendobj\n")
            .into_bytes();
        let first_xref = bytes.len();
        bytes.extend_from_slice(
            format!(
                "xref\n8 1\n0000000009 00000 n\r\ntrailer\n<< /Size 9 /Root 1 0 R /Info 8 0 R \
                 /Prev {main_xref:010} >>\nstartxref\n1\n%%EOF\n"
            )
            .as_bytes(),
        );
        let root = bytes.len();
        bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        let main_offset = bytes.len();
        bytes.extend_from_slice(
            format!(
                "xref\n1 1\n{root:010} 00000 n\r\ntrailer\n<< /Size 9 \
                 >>\nstartxref\n{first_xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        (bytes, main_offset)
    }

    fn append_classic_xref(bytes: &mut Vec<u8>, subsections: &str, trailer: &str) {
        let start = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n{subsections}trailer\n<< {trailer} >>\nstartxref\n{start}\n%%EOF\n")
                .as_bytes(),
        );
    }

    fn test_key(number: u32, generation: u16) -> Option<ObjectKey> {
        NonZeroU32::new(number).map(|number| ObjectKey::new(number, generation))
    }

    fn integer_object(parsed: &ParsedDocument, number: u32, generation: u16) -> Option<i64> {
        let value = &parsed.objects.get(&test_key(number, generation)?)?.object;
        match value {
            CosObject::Integer(value) => Some(*value),
            _ => None,
        }
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

    #[cfg(feature = "flate")]
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

    #[cfg(feature = "flate")]
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

    #[cfg(not(feature = "flate"))]
    #[test]
    fn test_should_report_unsupported_flate_when_disabled() -> crate::Result<()> {
        let document =
            Parser::default().parse(Cursor::new(single_stream_pdf("FlateDecode", "", b"")))?;
        let stream = parsed_stream(&document)?;
        assert!(matches!(
            stream.decoded_bytes(&ResourceLimits::default()),
            Err(ParseError::UnsupportedFilter { filter }) if filter.as_str() == "FlateDecode"
        ));
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
