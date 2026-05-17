#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
//! Public contracts for the pdfv validation engine.
//!
//! The crate currently exposes the stable data model, error model, and JSON
//! report writing spine used by later parser and validator phases.
//!
//! ```
//! use pdfv_core::{InputKind, InputSummary, ValidationOptions};
//!
//! let options = ValidationOptions::default();
//! let source = InputSummary::new(InputKind::Memory, None, None);
//! assert!(options.report_parse_warnings);
//! assert_eq!(source.kind, InputKind::Memory);
//! ```

mod content;
mod generated_profiles;
mod parser;
mod profile;
mod validation;
mod xmp;

use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
    time::Duration,
};

pub use parser::{
    CosObject, DecodeParams, DecoderRegistry, Dictionary, IndirectObject, ObjectStore,
    ParseOptions, ParsedDocument, Parser, PdfName, PdfSource, PdfString, SourceStorage,
    StreamDecoder, StreamObject, Trailer,
};
#[cfg(feature = "custom-profiles")]
pub use profile::CustomProfileRepository;
pub use profile::{
    BinaryOp, BuiltinFunction, BuiltinProfileRepository, ErrorTemplate, ModelSchemaParityReport,
    ModelSchemaProfileReport, ModelValue, ObjectTypeName, ProfileCatalogEntry,
    ProfileImportSummary, ProfileRepository, PropertyName, PropertyPath, Rule, RuleEvaluator,
    RuleExpr, RuleOutcome, UnaryOp, ValidationProfile, display_flavour, import_verapdf_profile_xml,
    model_schema_parity_report,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_builder::TypedBuilder;
pub use validation::{
    AnnotationModel, CatalogModel, ContentStreamModel, FeatureSelection, FontModel,
    InlineImageModel, InputName, LinkName, MarkedContentModel, MetadataModel, ModelGraph,
    ModelObject, ModelObjectRef, ObjectIdentity, OperatorModel, OutputIntentModel, PageModel,
    ResourceUseModel, Validator,
};
pub use xmp::{
    DetectedFlavours, FlavourClaim, FlavourDetector, NamespaceBinding, XmpIdentificationKind,
    XmpPacket, XmpParser,
};

/// Current library version embedded in generated reports.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 4096;
const DEFAULT_MAX_PASSWORD_BYTES: usize = 1024;
const HARD_MAX_PASSWORD_BYTES: usize = 4096;
const DEFAULT_MAX_STRING_BYTES: usize = 1_048_576;
const DEFAULT_MAX_STREAM_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_MAX_ENCRYPTION_DICT_ENTRIES: u64 = 64;
const DEFAULT_MEMORY_SOURCE_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_RESOURCE_CONTEXTS: u64 = 10_000;
const DEFAULT_MAX_EMBEDDED_FONT_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_ICC_PROFILE_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_MAX_XMP_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_MAX_XMP_ELEMENTS: u64 = 50_000;
const DEFAULT_MAX_XMP_DEPTH: u32 = 32;
const DEFAULT_MAX_XMP_ATTRIBUTES: usize = 64;
const DEFAULT_MAX_XMP_NAMESPACES: usize = 256;
const DEFAULT_MAX_XMP_TEXT_BYTES: usize = 4096;

/// Result alias for pdfv library operations.
pub type Result<T> = std::result::Result<T, PdfvError>;

/// Top-level library error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PdfvError {
    /// Input/output failure.
    #[error("I/O error{path}: {source}", path = format_optional_path(.path.as_ref()))]
    Io {
        /// Path associated with the failure when available.
        path: Option<PathBuf>,
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Parser failure.
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    /// Profile loading or selection failure.
    #[error("profile error: {0}")]
    Profile(#[from] ProfileError),
    /// Validation engine failure.
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
    /// Policy loading or evaluation failure.
    #[error("policy error: {0}")]
    Policy(#[from] PolicyError),
    /// Metadata repair failure.
    #[error("repair error: {0}")]
    Repair(#[from] RepairError),
    /// Report serialization failure.
    #[error("report error: {0}")]
    Report(#[from] ReportError),
    /// Configuration failure.
    #[error("configuration error: {0}")]
    Configuration(#[from] ConfigError),
}

/// Parser-specific error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParseError {
    /// A configured parser resource limit was exceeded.
    #[error("resource limit exceeded: {limit}")]
    LimitExceeded {
        /// Name of the exceeded limit.
        limit: &'static str,
    },
    /// Checked arithmetic overflowed while processing input.
    #[error("arithmetic overflow while parsing {context}")]
    ArithmeticOverflow {
        /// Parsing context that overflowed.
        context: &'static str,
    },
    /// PDF syntax could not be recovered.
    #[error("malformed PDF syntax: {message}")]
    Malformed {
        /// Bounded diagnostic message.
        message: BoundedText,
    },
    /// A referenced object was missing or had the wrong shape.
    #[error("missing PDF object: {message}")]
    MissingObject {
        /// Bounded diagnostic message.
        message: BoundedText,
    },
    /// A stream filter is not supported by this phase.
    #[error("unsupported stream filter: {filter}")]
    UnsupportedFilter {
        /// Filter name.
        filter: BoundedText,
    },
    /// Stream decoding failed.
    #[error("stream decode failed: {message}")]
    StreamDecode {
        /// Bounded diagnostic message.
        message: BoundedText,
    },
}

/// Profile-specific error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProfileError {
    /// Profile selection did not resolve to a supported profile.
    #[error("unsupported profile selection")]
    UnsupportedSelection,
    /// A profile field failed validation.
    #[error("invalid profile field {field}: {reason}")]
    InvalidField {
        /// Field that failed validation.
        field: &'static str,
        /// Bounded reason string.
        reason: BoundedText,
    },
    /// A rule expression exceeded a configured evaluation budget.
    #[error("rule evaluation budget exceeded: {budget}")]
    BudgetExceeded {
        /// Budget that was exceeded.
        budget: &'static str,
    },
    /// A rule referenced a property that does not exist on the model object.
    #[error("unknown model property {property}")]
    UnknownProperty {
        /// Property name.
        property: BoundedText,
    },
    /// A rule expression had a type mismatch.
    #[error("rule expression type mismatch: {message}")]
    TypeMismatch {
        /// Bounded diagnostic message.
        message: BoundedText,
    },
    /// A rule expression is not supported by the bounded IR.
    #[error("unsupported rule expression: {reason}")]
    UnsupportedRule {
        /// Bounded reason string.
        reason: BoundedText,
    },
    /// Profile XML failed bounded parsing.
    #[error("invalid profile XML: {reason}")]
    InvalidXml {
        /// Bounded reason string.
        reason: BoundedText,
    },
}

/// Validation-specific error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValidationError {
    /// Validation could not complete because a required subsystem is unavailable.
    #[error("validation subsystem is unavailable: {subsystem}")]
    SubsystemUnavailable {
        /// Subsystem name.
        subsystem: &'static str,
    },
    /// Validation traversal exceeded a configured resource limit.
    #[error("validation traversal limit exceeded: {limit}")]
    LimitExceeded {
        /// Limit that was exceeded.
        limit: &'static str,
    },
}

/// Feature policy error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum PolicyError {
    /// A policy document field failed validation.
    #[error("invalid policy field {field}: {reason}")]
    InvalidField {
        /// Field that failed validation.
        field: &'static str,
        /// Bounded reason string.
        reason: BoundedText,
    },
    /// A policy rule could not be evaluated against the feature report.
    #[error("policy rule could not be evaluated: {reason}")]
    Evaluation {
        /// Bounded reason string.
        reason: BoundedText,
    },
}

/// Metadata repair error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum RepairError {
    /// A repair option failed validation.
    #[error("invalid repair field {field}: {reason}")]
    InvalidField {
        /// Field that failed validation.
        field: &'static str,
        /// Bounded reason string.
        reason: BoundedText,
    },
    /// A repair operation could not be completed.
    #[error("metadata repair failed: {reason}")]
    Failed {
        /// Bounded reason string.
        reason: BoundedText,
    },
}

/// Reporting-specific error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReportError {
    /// JSON serialization failed.
    #[error("JSON serialization failed")]
    Json {
        /// Source JSON error.
        #[from]
        source: serde_json::Error,
    },
    /// XML serialization failed.
    #[error("XML serialization failed: {message}")]
    Xml {
        /// Bounded diagnostic message.
        message: BoundedText,
    },
    /// Output write failed.
    #[error("report output write failed")]
    Write {
        /// Source I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Configuration-specific error.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configured value was outside the accepted range.
    #[error("invalid configuration value {field}: {reason}")]
    InvalidValue {
        /// Configuration field name.
        field: &'static str,
        /// Bounded reason string.
        reason: BoundedText,
    },
}

/// Bounded UTF-8 text for externally supplied strings.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct BoundedText(String);

impl BoundedText {
    /// Creates bounded text with a maximum byte length.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when `value` is longer than `max_bytes`.
    pub fn new(
        value: impl Into<String>,
        max_bytes: usize,
    ) -> std::result::Result<Self, ConfigError> {
        let value = value.into();
        if value.len() > max_bytes {
            return Err(ConfigError::InvalidValue {
                field: "text",
                reason: Self::unchecked("value exceeds byte limit"),
            });
        }
        Ok(Self(value))
    }

    /// Returns the text as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for BoundedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for BoundedText {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value, MAX_TEXT_BYTES)
    }
}

impl From<BoundedText> for String {
    fn from(value: BoundedText) -> Self {
        value.0
    }
}

/// Identifier text with a tight byte cap and ASCII policy.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct Identifier(String);

impl Identifier {
    /// Creates an identifier from ASCII alphanumeric, dash, underscore, dot, and colon characters.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the identifier is empty, too long, or contains
    /// characters outside the allowlist.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ConfigError> {
        let value = value.into();
        let valid_charset = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
        if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || !valid_charset {
            return Err(ConfigError::InvalidValue {
                field: "identifier",
                reason: BoundedText::unchecked("identifier violates byte or charset policy"),
            });
        }
        Ok(Self(value))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn unchecked(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl TryFrom<String> for Identifier {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Identifier> for String {
    fn from(value: Identifier) -> Self {
        value.0
    }
}

/// Redacted PDF password secret.
#[derive(Clone)]
pub struct PasswordSecret(SecretString);

impl PasswordSecret {
    /// Creates a password secret using the default password byte cap.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the password exceeds the default cap.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ConfigError> {
        Self::new_with_limit(value, DEFAULT_MAX_PASSWORD_BYTES)
    }

    /// Creates a password secret using an explicit byte cap.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the password exceeds the supplied cap or the
    /// cap is above the hard limit.
    pub fn new_with_limit(
        value: impl Into<String>,
        max_bytes: usize,
    ) -> std::result::Result<Self, ConfigError> {
        if max_bytes > HARD_MAX_PASSWORD_BYTES {
            return Err(ConfigError::InvalidValue {
                field: "maxPasswordBytes",
                reason: BoundedText::unchecked("value exceeds hard cap"),
            });
        }
        let value = value.into();
        if value.len() > max_bytes {
            return Err(ConfigError::InvalidValue {
                field: "password",
                reason: BoundedText::unchecked("password exceeds byte limit"),
            });
        }
        Ok(Self(SecretString::from(value)))
    }

    pub(crate) fn expose_secret_bytes(&self) -> &[u8] {
        self.0.expose_secret().as_bytes()
    }
}

impl fmt::Debug for PasswordSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PasswordSecret([REDACTED])")
    }
}

/// PDF validation options shared by parser, engine, and reports.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationOptions {
    /// Flavour/profile selection policy.
    #[builder(default)]
    pub flavour: FlavourSelection,
    /// Parser and validation resource limits.
    #[builder(default)]
    pub resource_limits: ResourceLimits,
    /// Optional redacted password for encrypted PDFs.
    #[builder(default)]
    #[serde(skip, default)]
    pub password: Option<PasswordSecret>,
    /// Maximum assertion details retained per failed rule.
    #[builder(default)]
    pub max_failed_assertions_per_rule: MaxDisplayedFailures,
    /// Whether passed assertion details are recorded.
    #[builder(default)]
    pub record_passed_assertions: bool,
    /// Whether recoverable parser warnings are included in the report.
    #[builder(default = true)]
    pub report_parse_warnings: bool,
    /// Optional feature families to extract into reports.
    #[builder(default)]
    pub feature_selection: FeatureSelection,
    /// Optional feature-policy rules to evaluate.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicySet>,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// Flavour/profile selection policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FlavourSelection {
    /// Detect flavour from document metadata, optionally falling back to a default.
    Auto {
        /// Default flavour used when auto-detection is inconclusive.
        default: Option<ValidationFlavour>,
    },
    /// Validate against an explicit built-in flavour.
    Explicit {
        /// Selected validation flavour.
        flavour: ValidationFlavour,
    },
    /// Validate against a custom profile loaded from a path.
    CustomProfile {
        /// Custom profile file path.
        profile_path: PathBuf,
    },
}

impl Default for FlavourSelection {
    fn default() -> Self {
        Self::Auto {
            default: Some(ValidationFlavour {
                family: Identifier::unchecked("pdfa"),
                part: NonZeroU32::MIN,
                conformance: Identifier::unchecked("b"),
            }),
        }
    }
}

/// Validation flavour identifier.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationFlavour {
    /// PDF family, such as `pdfa`.
    pub family: Identifier,
    /// Part number, such as `1`.
    pub part: NonZeroU32,
    /// Conformance level, such as `b`.
    pub conformance: Identifier,
}

impl ValidationFlavour {
    /// Creates a validation flavour.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when identifier fields violate the identifier policy.
    pub fn new(
        family: impl Into<String>,
        part: NonZeroU32,
        conformance: impl Into<String>,
    ) -> std::result::Result<Self, ConfigError> {
        Ok(Self {
            family: Identifier::new(family)?,
            part,
            conformance: Identifier::new(conformance)?,
        })
    }
}

/// Parser and validation resource limits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceLimits {
    /// Maximum input file bytes.
    pub max_file_bytes: u64,
    /// Maximum indirect objects.
    pub max_objects: u64,
    /// Maximum nested object depth.
    pub max_object_depth: u32,
    /// Maximum array length.
    pub max_array_len: u64,
    /// Maximum dictionary entries.
    pub max_dict_entries: u64,
    /// Maximum PDF name bytes.
    pub max_name_bytes: usize,
    /// Maximum string bytes.
    pub max_string_bytes: usize,
    /// Maximum password bytes accepted from public APIs and CLI sources.
    #[builder(default = DEFAULT_MAX_PASSWORD_BYTES)]
    #[serde(default = "default_max_password_bytes")]
    pub max_password_bytes: usize,
    /// Maximum decrypted string bytes.
    #[builder(default = DEFAULT_MAX_STRING_BYTES)]
    #[serde(default = "default_max_decrypted_string_bytes")]
    pub max_decrypted_string_bytes: usize,
    /// Maximum declared stream bytes.
    pub max_stream_declared_bytes: u64,
    /// Maximum decoded stream bytes.
    pub max_stream_decode_bytes: u64,
    /// Maximum decrypted stream bytes before downstream filters.
    #[builder(default = DEFAULT_MAX_STREAM_DECODE_BYTES)]
    #[serde(default = "default_max_decrypted_stream_bytes")]
    pub max_decrypted_stream_bytes: u64,
    /// Maximum encryption dictionary entries.
    #[builder(default = DEFAULT_MAX_ENCRYPTION_DICT_ENTRIES)]
    #[serde(default = "default_max_encryption_dict_entries")]
    pub max_encryption_dict_entries: u64,
    /// Maximum source bytes kept in memory before spilling to a temporary file.
    #[builder(default = DEFAULT_MEMORY_SOURCE_THRESHOLD_BYTES)]
    #[serde(default = "default_memory_source_threshold_bytes")]
    pub memory_source_threshold_bytes: u64,
    /// Maximum retained parse facts.
    pub max_parse_facts: usize,
    /// Maximum catalog XMP metadata stream bytes.
    #[builder(default = DEFAULT_MAX_XMP_BYTES)]
    #[serde(default = "default_max_xmp_bytes")]
    pub max_xmp_bytes: u64,
    /// Maximum XML elements parsed from one XMP packet.
    #[builder(default = DEFAULT_MAX_XMP_ELEMENTS)]
    #[serde(default = "default_max_xmp_elements")]
    pub max_xmp_elements: u64,
    /// Maximum XML element nesting depth in one XMP packet.
    #[builder(default = DEFAULT_MAX_XMP_DEPTH)]
    #[serde(default = "default_max_xmp_depth")]
    pub max_xmp_depth: u32,
    /// Maximum XML attributes accepted on one XMP element.
    #[builder(default = DEFAULT_MAX_XMP_ATTRIBUTES)]
    #[serde(default = "default_max_xmp_attributes")]
    pub max_xmp_attributes: usize,
    /// Maximum namespace declarations retained from one XMP packet.
    #[builder(default = DEFAULT_MAX_XMP_NAMESPACES)]
    #[serde(default = "default_max_xmp_namespaces")]
    pub max_xmp_namespaces: usize,
    /// Maximum text bytes retained from one XMP metadata property.
    #[builder(default = DEFAULT_MAX_XMP_TEXT_BYTES)]
    #[serde(default = "default_max_xmp_text_bytes")]
    pub max_xmp_text_bytes: usize,
    /// Maximum content-stream operators parsed per stream.
    #[builder(default = content::default_max_content_stream_ops())]
    #[serde(default = "content::default_max_content_stream_ops")]
    pub max_content_stream_ops: u64,
    /// Maximum content-stream operands retained for one operator.
    #[builder(default = content::default_max_content_stream_operand_count())]
    #[serde(default = "content::default_max_content_stream_operand_count")]
    pub max_content_stream_operand_count: usize,
    /// Maximum serialized operand bytes retained for one operator.
    #[builder(default = content::default_max_content_stream_operand_bytes())]
    #[serde(default = "content::default_max_content_stream_operand_bytes")]
    pub max_content_stream_operand_bytes: usize,
    /// Maximum content-stream operator name bytes.
    #[builder(default = content::default_max_content_stream_operator_name_bytes())]
    #[serde(default = "content::default_max_content_stream_operator_name_bytes")]
    pub max_content_stream_operator_name_bytes: usize,
    /// Maximum inline image data bytes scanned inside one content stream.
    #[builder(default = content::default_max_inline_image_bytes())]
    #[serde(default = "content::default_max_inline_image_bytes")]
    pub max_inline_image_bytes: u64,
    /// Maximum tracked graphics-state stack depth.
    #[builder(default = content::default_max_graphics_state_depth())]
    #[serde(default = "content::default_max_graphics_state_depth")]
    pub max_graphics_state_depth: u32,
    /// Maximum Type 3 charproc streams parsed per validation session.
    #[builder(default = content::default_max_type3_charproc_streams())]
    #[serde(default = "content::default_max_type3_charproc_streams")]
    pub max_type3_charproc_streams: u64,
    /// Maximum operators parsed from one Type 3 charproc stream.
    #[builder(default = content::default_max_type3_charproc_ops())]
    #[serde(default = "content::default_max_type3_charproc_ops")]
    pub max_type3_charproc_ops: u64,
    /// Maximum resource dictionaries traversed while resolving one effective resource context.
    #[builder(default = DEFAULT_MAX_RESOURCE_CONTEXTS)]
    #[serde(default = "default_max_resource_contexts")]
    pub max_resource_contexts: u64,
    /// Maximum embedded font program bytes summarized for validation facts.
    #[builder(default = DEFAULT_MAX_EMBEDDED_FONT_BYTES)]
    #[serde(default = "default_max_embedded_font_bytes")]
    pub max_embedded_font_bytes: u64,
    /// Maximum ICC profile bytes summarized for validation facts.
    #[builder(default = DEFAULT_MAX_ICC_PROFILE_BYTES)]
    #[serde(default = "default_max_icc_profile_bytes")]
    pub max_icc_profile_bytes: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024 * 1024,
            max_objects: 1_000_000,
            max_object_depth: 128,
            max_array_len: 65_536,
            max_dict_entries: 16_384,
            max_name_bytes: 127,
            max_string_bytes: DEFAULT_MAX_STRING_BYTES,
            max_password_bytes: DEFAULT_MAX_PASSWORD_BYTES,
            max_decrypted_string_bytes: DEFAULT_MAX_STRING_BYTES,
            max_stream_declared_bytes: 128 * 1024 * 1024,
            max_stream_decode_bytes: DEFAULT_MAX_STREAM_DECODE_BYTES,
            max_decrypted_stream_bytes: DEFAULT_MAX_STREAM_DECODE_BYTES,
            max_encryption_dict_entries: DEFAULT_MAX_ENCRYPTION_DICT_ENTRIES,
            memory_source_threshold_bytes: DEFAULT_MEMORY_SOURCE_THRESHOLD_BYTES,
            max_parse_facts: 100_000,
            max_xmp_bytes: DEFAULT_MAX_XMP_BYTES,
            max_xmp_elements: DEFAULT_MAX_XMP_ELEMENTS,
            max_xmp_depth: DEFAULT_MAX_XMP_DEPTH,
            max_xmp_attributes: DEFAULT_MAX_XMP_ATTRIBUTES,
            max_xmp_namespaces: DEFAULT_MAX_XMP_NAMESPACES,
            max_xmp_text_bytes: DEFAULT_MAX_XMP_TEXT_BYTES,
            max_content_stream_ops: content::default_max_content_stream_ops(),
            max_content_stream_operand_count: content::default_max_content_stream_operand_count(),
            max_content_stream_operand_bytes: content::default_max_content_stream_operand_bytes(),
            max_content_stream_operator_name_bytes:
                content::default_max_content_stream_operator_name_bytes(),
            max_inline_image_bytes: content::default_max_inline_image_bytes(),
            max_graphics_state_depth: content::default_max_graphics_state_depth(),
            max_type3_charproc_streams: content::default_max_type3_charproc_streams(),
            max_type3_charproc_ops: content::default_max_type3_charproc_ops(),
            max_resource_contexts: DEFAULT_MAX_RESOURCE_CONTEXTS,
            max_embedded_font_bytes: DEFAULT_MAX_EMBEDDED_FONT_BYTES,
            max_icc_profile_bytes: DEFAULT_MAX_ICC_PROFILE_BYTES,
        }
    }
}

fn default_max_password_bytes() -> usize {
    DEFAULT_MAX_PASSWORD_BYTES
}

fn default_max_decrypted_string_bytes() -> usize {
    DEFAULT_MAX_STRING_BYTES
}

fn default_max_decrypted_stream_bytes() -> u64 {
    DEFAULT_MAX_STREAM_DECODE_BYTES
}

fn default_max_resource_contexts() -> u64 {
    DEFAULT_MAX_RESOURCE_CONTEXTS
}

fn default_max_embedded_font_bytes() -> u64 {
    DEFAULT_MAX_EMBEDDED_FONT_BYTES
}

fn default_max_icc_profile_bytes() -> u64 {
    DEFAULT_MAX_ICC_PROFILE_BYTES
}

fn default_max_encryption_dict_entries() -> u64 {
    DEFAULT_MAX_ENCRYPTION_DICT_ENTRIES
}

fn default_memory_source_threshold_bytes() -> u64 {
    DEFAULT_MEMORY_SOURCE_THRESHOLD_BYTES
}

fn default_max_xmp_bytes() -> u64 {
    DEFAULT_MAX_XMP_BYTES
}

fn default_max_xmp_elements() -> u64 {
    DEFAULT_MAX_XMP_ELEMENTS
}

fn default_max_xmp_depth() -> u32 {
    DEFAULT_MAX_XMP_DEPTH
}

fn default_max_xmp_attributes() -> usize {
    DEFAULT_MAX_XMP_ATTRIBUTES
}

fn default_max_xmp_namespaces() -> usize {
    DEFAULT_MAX_XMP_NAMESPACES
}

fn default_max_xmp_text_bytes() -> usize {
    DEFAULT_MAX_XMP_TEXT_BYTES
}

/// Maximum displayed assertion failures per rule.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct MaxDisplayedFailures(NonZeroU32);

impl MaxDisplayedFailures {
    /// Creates a failure display cap.
    #[must_use]
    pub fn new(value: NonZeroU32) -> Self {
        Self(value)
    }

    /// Returns the cap as `u32`.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl Default for MaxDisplayedFailures {
    fn default() -> Self {
        Self(NonZeroU32::MIN)
    }
}

impl TryFrom<u32> for MaxDisplayedFailures {
    type Error = ConfigError;

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        let Some(value) = NonZeroU32::new(value) else {
            return Err(ConfigError::InvalidValue {
                field: "maxFailedAssertionsPerRule",
                reason: BoundedText::unchecked("value must be greater than zero"),
            });
        };
        Ok(Self(value))
    }
}

impl From<MaxDisplayedFailures> for u32 {
    fn from(value: MaxDisplayedFailures) -> Self {
        value.get()
    }
}

/// Complete validation report for one input.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationReport {
    /// Engine version that produced the report.
    pub engine_version: String,
    /// Input summary.
    pub source: InputSummary,
    /// Overall validation status.
    pub status: ValidationStatus,
    /// Detected or selected flavours.
    pub flavours: Vec<ValidationFlavour>,
    /// Per-profile validation results.
    pub profile_reports: Vec<ProfileReport>,
    /// Parser facts retained for validation and diagnostics.
    pub parse_facts: Vec<ParseFact>,
    /// User-visible warnings.
    pub warnings: Vec<ValidationWarning>,
    /// Optional read-only feature extraction report.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_report: Option<FeatureReport>,
    /// Optional policy evaluation report.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_report: Option<PolicyReport>,
    /// Task duration measurements.
    pub task_durations: Vec<TaskDuration>,
}

/// Machine-readable read-only feature extraction report.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureReport {
    /// Extracted feature objects in deterministic traversal order.
    pub objects: Vec<FeatureObject>,
    /// Total model objects visited while extracting features.
    pub visited_objects: u64,
    /// Feature families requested by the caller.
    pub selected_families: Vec<ObjectTypeName>,
    /// Whether extraction stopped because a resource limit was reached.
    pub truncated: bool,
}

/// One extracted validation-model object.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureObject {
    /// Validation model family name.
    pub family: ObjectTypeName,
    /// Stable object location.
    pub location: ObjectLocation,
    /// Bounded diagnostic context path.
    pub context: BoundedText,
    /// Extracted scalar properties keyed by validation-model property name.
    pub properties: BTreeMap<PropertyName, FeatureValue>,
}

/// Feature property value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum FeatureValue {
    /// Null feature value.
    Null,
    /// Boolean feature value.
    Bool(bool),
    /// Numeric feature value.
    Number(f64),
    /// Bounded string feature value.
    String(BoundedText),
    /// Content-bearing string value redacted from reports.
    RedactedString {
        /// Original string byte length.
        bytes: u64,
    },
    /// Object key feature value.
    ObjectKey(ObjectKey),
    /// Bounded list feature value.
    List(Vec<FeatureValue>),
}

/// Bounded policy rules evaluated over a [`FeatureReport`].
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicySet {
    /// Optional policy document name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<BoundedText>,
    /// Policy rules.
    pub rules: Vec<PolicyRule>,
}

impl PolicySet {
    /// Validates collection-level policy limits.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError`] when the policy exceeds compiled limits.
    pub fn validate(&self) -> std::result::Result<(), PolicyError> {
        const MAX_POLICY_RULES: usize = 1024;
        if self.rules.is_empty() {
            return Err(PolicyError::InvalidField {
                field: "rules",
                reason: BoundedText::unchecked("policy must contain at least one rule"),
            });
        }
        if self.rules.len() > MAX_POLICY_RULES {
            return Err(PolicyError::InvalidField {
                field: "rules",
                reason: BoundedText::unchecked("policy rule count exceeds limit"),
            });
        }
        Ok(())
    }
}

/// One bounded policy rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyRule {
    /// Rule identifier.
    pub id: Identifier,
    /// Human-readable rule description.
    pub description: BoundedText,
    /// Feature family to inspect.
    pub family: ObjectTypeName,
    /// Feature property to inspect.
    pub field: PropertyName,
    /// Comparison operator.
    pub operator: PolicyOperator,
    /// Optional comparison value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<PolicyValue>,
}

/// Bounded policy comparison operator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum PolicyOperator {
    /// At least one matching feature object contains the field.
    Exists,
    /// No matching feature object contains the field.
    Absent,
    /// At least one matching value equals the rule value.
    Equals,
    /// No matching value equals the rule value.
    NotEquals,
    /// At least one matching numeric value is greater than or equal to the rule value.
    Min,
    /// At least one matching numeric value is less than or equal to the rule value.
    Max,
}

/// Policy comparison value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum PolicyValue {
    /// Boolean comparison value.
    Bool(bool),
    /// Integer comparison value.
    Number(i32),
    /// Bounded string comparison value.
    String(BoundedText),
}

/// Policy evaluation report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyReport {
    /// Optional policy document name.
    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<BoundedText>,
    /// Whether all policy rules passed.
    pub is_compliant: bool,
    /// Rule results.
    pub results: Vec<PolicyRuleResult>,
}

/// One policy rule result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyRuleResult {
    /// Rule identifier.
    pub id: Identifier,
    /// Human-readable rule description.
    pub description: BoundedText,
    /// Rule pass/fail status.
    pub passed: bool,
    /// Number of matching feature objects considered.
    pub matches: u64,
    /// Bounded diagnostic message.
    pub message: BoundedText,
}

/// Metadata repair report for one input.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairReport {
    /// Engine version that produced the report.
    pub engine_version: String,
    /// Input summary.
    pub source: InputSummary,
    /// Optional output path when a repaired or unchanged file was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<PathBuf>,
    /// Overall repair status.
    pub status: RepairStatus,
    /// Actions completed for this input.
    pub actions: Vec<RepairAction>,
    /// Refusal reason when no output was produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<RepairRefusal>,
    /// User-visible repair warnings.
    pub warnings: Vec<ValidationWarning>,
    /// Task duration measurements.
    pub task_durations: Vec<TaskDuration>,
}

impl RepairReport {
    /// Returns true when the report describes a written output file.
    #[must_use]
    pub fn wrote_output(&self) -> bool {
        matches!(
            self.status,
            RepairStatus::Succeeded | RepairStatus::NoAction
        ) && self.output_path.is_some()
    }
}

/// Options for safe metadata repair.
#[derive(Clone, Debug)]
pub struct MetadataRepairOptions {
    /// Validation options used to parse and classify repair inputs.
    pub validation_options: ValidationOptions,
    /// Canonical output directory where repaired files are written.
    pub output_dir: PathBuf,
    /// Prefix added to each output filename.
    pub prefix: String,
}

impl MetadataRepairOptions {
    /// Creates repair options after validating output directory and prefix.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when the output directory or prefix violates the
    /// repair safety policy.
    pub fn new(
        validation_options: ValidationOptions,
        output_dir: impl AsRef<Path>,
        prefix: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            validation_options,
            output_dir: validate_repair_output_dir(output_dir.as_ref())?,
            prefix: validate_repair_prefix(&prefix.into())?,
        })
    }
}

/// Safe metadata repair facade.
#[derive(Debug)]
pub struct MetadataRepairer {
    validator: Validator,
    output_dir: PathBuf,
    prefix: String,
}

impl MetadataRepairer {
    /// Creates a metadata repair facade.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if validation setup or repair options are invalid.
    pub fn new(options: MetadataRepairOptions) -> Result<Self> {
        Ok(Self {
            validator: Validator::new(options.validation_options)?,
            output_dir: options.output_dir,
            prefix: options.prefix,
        })
    }

    /// Repairs one PDF file by writing a non-in-place output or a refusal report.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] for I/O failures before a report can be produced.
    pub fn repair_path(&self, path: impl AsRef<Path>) -> Result<RepairReport> {
        repair_metadata_path(
            &self.validator,
            path.as_ref(),
            &self.output_dir,
            &self.prefix,
        )
    }
}

/// Batch metadata repair report.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairBatchReport {
    /// Item reports.
    pub items: Vec<RepairReport>,
    /// Batch summary.
    pub summary: RepairBatchSummary,
    /// Batch-level warnings.
    pub warnings: Vec<ValidationWarning>,
}

impl RepairBatchReport {
    /// Builds a repair batch report and computes summary counters.
    #[must_use]
    pub fn from_items(
        items: Vec<RepairReport>,
        warnings: Vec<ValidationWarning>,
        elapsed: Duration,
    ) -> Self {
        let summary = RepairBatchSummary::from_items(&items, elapsed);
        Self {
            items,
            summary,
            warnings,
        }
    }
}

/// Batch metadata repair summary counters.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairBatchSummary {
    /// Total input count.
    pub total_files: u64,
    /// Inputs that produced a modified repair output.
    pub succeeded: u64,
    /// Inputs that needed no metadata change and were copied unchanged.
    pub no_action: u64,
    /// Inputs refused by the repair safety model.
    pub refused: u64,
    /// Inputs that failed while attempting an output write.
    pub failed: u64,
    /// Elapsed milliseconds.
    pub elapsed_millis: u64,
    /// Worst exit category.
    pub worst_exit_category: ExitCategory,
}

impl RepairBatchSummary {
    /// Computes summary counters from item reports.
    #[must_use]
    pub fn from_items(items: &[RepairReport], elapsed: Duration) -> Self {
        let mut summary = Self {
            total_files: u64::try_from(items.len()).unwrap_or(u64::MAX),
            elapsed_millis: duration_millis(elapsed),
            ..Self::default()
        };
        for item in items {
            match item.status {
                RepairStatus::Succeeded => summary.succeeded = summary.succeeded.saturating_add(1),
                RepairStatus::NoAction => summary.no_action = summary.no_action.saturating_add(1),
                RepairStatus::Refused => summary.refused = summary.refused.saturating_add(1),
                RepairStatus::Failed => summary.failed = summary.failed.saturating_add(1),
            }
        }
        summary.worst_exit_category = if summary.failed > 0 {
            ExitCategory::InternalError
        } else if summary.refused > 0 {
            ExitCategory::ProcessingFailed
        } else {
            ExitCategory::Success
        };
        summary
    }
}

/// Metadata repair status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum RepairStatus {
    /// Repair modified metadata and wrote an output file.
    Succeeded,
    /// No metadata change was needed; an unchanged output file was written.
    NoAction,
    /// Repair was explicitly refused before writing output.
    Refused,
    /// Repair failed while writing or finalizing output.
    Failed,
}

/// Metadata repair action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RepairAction {
    /// The input was copied unchanged to the output path.
    CopiedUnchanged,
    /// XMP metadata was repaired.
    MetadataRewritten {
        /// Bounded action description.
        description: BoundedText,
    },
}

/// Explicit reason metadata repair was refused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RepairRefusal {
    /// Input could not be parsed as PDF.
    ParseFailed {
        /// Bounded reason.
        reason: BoundedText,
    },
    /// Encrypted inputs are not repaired by this phase.
    Encrypted,
    /// Repair requires exactly one selected validation flavour.
    AmbiguousFlavour {
        /// Number of selected flavours.
        selected: u64,
    },
    /// Validation failed and safe metadata rewrite support is unavailable.
    UnsupportedValidationStatus {
        /// Validation status that blocked repair.
        status: ValidationStatus,
    },
    /// Output path would overwrite the input.
    OutputWouldModifyInput,
    /// Output path failed validation.
    InvalidOutputPath {
        /// Bounded reason.
        reason: BoundedText,
    },
}

#[allow(
    clippy::disallowed_methods,
    reason = "metadata repair is an explicit synchronous file rewrite API, not an async service \
              path"
)]
fn repair_metadata_path(
    validator: &Validator,
    path: &Path,
    output_dir: &Path,
    prefix: &str,
) -> Result<RepairReport> {
    let source = input_summary_for_path(path)?;
    let output_path = repair_output_path(path, output_dir, prefix)?;
    let input_canonical = std::fs::canonicalize(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    if input_canonical == output_path {
        return Ok(refused_repair_report(
            source,
            RepairRefusal::OutputWouldModifyInput,
        ));
    }
    if output_path.exists() {
        return Ok(refused_repair_report(
            source,
            RepairRefusal::InvalidOutputPath {
                reason: BoundedText::unchecked("output path already exists"),
            },
        ));
    }

    let started = std::time::Instant::now();
    let validation = validator.validate_path(path)?;
    if matches!(validation.status, ValidationStatus::ParseFailed) {
        return Ok(refused_repair_report(
            source,
            RepairRefusal::ParseFailed {
                reason: validation
                    .warnings
                    .first()
                    .map_or_else(default_parse_failed_text, ValidationWarning::message_text),
            },
        ));
    }
    if matches!(validation.status, ValidationStatus::Encrypted) {
        return Ok(refused_repair_report(source, RepairRefusal::Encrypted));
    }
    let selected_profiles = if validation.flavours.is_empty() {
        validation.profile_reports.len()
    } else {
        validation.flavours.len()
    };
    if selected_profiles != 1 {
        return Ok(refused_repair_report(
            source,
            RepairRefusal::AmbiguousFlavour {
                selected: u64::try_from(selected_profiles).unwrap_or(u64::MAX),
            },
        ));
    }
    if !matches!(validation.status, ValidationStatus::Valid) {
        return Ok(refused_repair_report(
            source,
            RepairRefusal::UnsupportedValidationStatus {
                status: validation.status,
            },
        ));
    }

    match atomic_copy(path, &output_path) {
        Ok(()) => Ok(RepairReport::builder()
            .engine_version(ENGINE_VERSION.to_owned())
            .source(source)
            .output_path(Some(output_path))
            .status(RepairStatus::NoAction)
            .actions(vec![RepairAction::CopiedUnchanged])
            .refusal(None)
            .warnings(Vec::new())
            .task_durations(vec![TaskDuration::from_duration(
                Identifier::new("repairMetadata")?,
                started.elapsed(),
            )])
            .build()),
        Err(error) => {
            remove_failed_output(&output_path)?;
            Ok(failed_repair_report(
                source,
                Some(output_path),
                &error.to_string(),
            ))
        }
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "metadata repair reports filesystem input size synchronously"
)]
fn input_summary_for_path(path: &Path) -> Result<InputSummary> {
    let metadata = std::fs::metadata(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    Ok(InputSummary::new(
        InputKind::File,
        Some(path.to_path_buf()),
        Some(metadata.len()),
    ))
}

#[allow(
    clippy::disallowed_methods,
    reason = "metadata repair validates a caller-selected filesystem output directory"
)]
fn validate_repair_output_dir(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::metadata(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(RepairError::InvalidField {
            field: "outputDir",
            reason: BoundedText::unchecked("output directory is not a directory"),
        }
        .into());
    }
    std::fs::canonicalize(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })
}

fn validate_repair_prefix(prefix: &str) -> Result<String> {
    const MAX_REPAIR_PREFIX_BYTES: usize = 64;
    let valid = prefix.len() <= MAX_REPAIR_PREFIX_BYTES
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(prefix.to_owned())
    } else {
        Err(RepairError::InvalidField {
            field: "prefix",
            reason: BoundedText::unchecked(
                "prefix must be ASCII letters, digits, dot, dash, or underscore and at most 64 \
                 bytes",
            ),
        }
        .into())
    }
}

fn repair_output_path(path: &Path, output_dir: &Path, prefix: &str) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RepairError::InvalidField {
            field: "paths",
            reason: BoundedText::unchecked("input path must have a UTF-8 file name"),
        })?;
    validate_output_filename(file_name)?;
    let output_name = format!("{prefix}{file_name}");
    validate_output_filename(&output_name)?;
    Ok(output_dir.join(output_name))
}

fn validate_output_filename(name: &str) -> Result<()> {
    const MAX_OUTPUT_FILENAME_BYTES: usize = 255;
    let valid = !name.is_empty()
        && name.len() <= MAX_OUTPUT_FILENAME_BYTES
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte != b'\0' && byte != b'/' && byte != b'\\');
    if valid {
        Ok(())
    } else {
        Err(RepairError::InvalidField {
            field: "output",
            reason: BoundedText::unchecked("output filename is invalid"),
        }
        .into())
    }
}

#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "metadata repair performs synchronous atomic file output by design"
)]
fn atomic_copy(input: &Path, output_path: &Path) -> Result<()> {
    let Some(parent) = output_path.parent() else {
        return Err(RepairError::InvalidField {
            field: "outputDir",
            reason: BoundedText::unchecked("output path has no parent"),
        }
        .into());
    };
    let mut source = std::fs::File::open(input).map_err(|source| PdfvError::Io {
        path: Some(input.to_path_buf()),
        source,
    })?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| PdfvError::Io {
        path: Some(parent.to_path_buf()),
        source,
    })?;
    io::copy(&mut source, &mut temp).map_err(|source| PdfvError::Io {
        path: Some(input.to_path_buf()),
        source,
    })?;
    temp.flush().map_err(|source| PdfvError::Io {
        path: Some(output_path.to_path_buf()),
        source,
    })?;
    temp.persist(output_path).map_err(|error| PdfvError::Io {
        path: Some(output_path.to_path_buf()),
        source: error.error,
    })?;
    Ok(())
}

#[allow(
    clippy::disallowed_methods,
    reason = "metadata repair removes failed synchronous output artifacts"
)]
fn remove_failed_output(output_path: &Path) -> Result<()> {
    match std::fs::remove_file(output_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PdfvError::Io {
            path: Some(output_path.to_path_buf()),
            source,
        }),
    }
}

fn refused_repair_report(source: InputSummary, refusal: RepairRefusal) -> RepairReport {
    RepairReport::builder()
        .engine_version(ENGINE_VERSION.to_owned())
        .source(source)
        .output_path(None)
        .status(RepairStatus::Refused)
        .actions(Vec::new())
        .refusal(Some(refusal))
        .warnings(Vec::new())
        .task_durations(Vec::new())
        .build()
}

fn failed_repair_report(
    source: InputSummary,
    output_path: Option<PathBuf>,
    reason: &str,
) -> RepairReport {
    RepairReport::builder()
        .engine_version(ENGINE_VERSION.to_owned())
        .source(source)
        .output_path(output_path)
        .status(RepairStatus::Failed)
        .actions(Vec::new())
        .refusal(None)
        .warnings(vec![ValidationWarning::General {
            message: BoundedText::new(reason, 512)
                .unwrap_or_else(|_| BoundedText::unchecked("metadata repair failed")),
        }])
        .task_durations(Vec::new())
        .build()
}

fn default_parse_failed_text() -> BoundedText {
    BoundedText::unchecked("parse failed")
}

/// Input summary included in reports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSummary {
    /// Input kind.
    pub kind: InputKind,
    /// Path when the input came from the filesystem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Input byte length when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

impl InputSummary {
    /// Creates an input summary.
    #[must_use]
    pub fn new(kind: InputKind, path: Option<PathBuf>, bytes: Option<u64>) -> Self {
        Self { kind, path, bytes }
    }
}

/// Input kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum InputKind {
    /// Filesystem input.
    File,
    /// In-memory or reader input.
    Memory,
}

/// Overall validation status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ValidationStatus {
    /// All required checks passed.
    Valid,
    /// One or more required checks failed.
    Invalid,
    /// Input is encrypted and cannot be validated in the current phase.
    Encrypted,
    /// Validation could not complete.
    Incomplete,
    /// Input could not be parsed.
    ParseFailed,
}

/// Per-profile report.
#[derive(Clone, Debug, Deserialize, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileReport {
    /// Profile identity.
    pub profile: ProfileIdentity,
    /// Whether this profile is compliant.
    pub is_compliant: bool,
    /// Number of checks executed.
    pub checks_executed: u64,
    /// Number of rules executed.
    pub rules_executed: u64,
    /// Number of failed rules.
    pub failed_rules: u64,
    /// Bounded failed assertion details.
    pub failed_assertions: Vec<Assertion>,
    /// Bounded passed assertion details.
    pub passed_assertions: Vec<Assertion>,
    /// Unsupported required rules.
    pub unsupported_rules: Vec<UnsupportedRule>,
}

/// Profile identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileIdentity {
    /// Profile id.
    pub id: Identifier,
    /// Human-readable profile name.
    pub name: BoundedText,
    /// Profile version string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Identifier>,
}

/// Rule assertion detail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assertion {
    /// Report-stable assertion ordinal.
    pub ordinal: NonZeroU64,
    /// Rule id.
    pub rule_id: RuleId,
    /// Assertion status.
    pub status: AssertionStatus,
    /// Assertion description.
    pub description: BoundedText,
    /// Object location.
    pub location: ObjectLocation,
    /// Optional object context path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_context: Option<BoundedText>,
    /// Optional assertion message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<BoundedText>,
    /// Error template arguments.
    pub error_arguments: Vec<ErrorArgument>,
}

/// Assertion status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum AssertionStatus {
    /// Assertion passed.
    Passed,
    /// Assertion failed.
    Failed,
}

/// Rule id.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RuleId(pub Identifier);

/// Object location for diagnostics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectLocation {
    /// Indirect object key when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectKey>,
    /// Byte offset when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// Human-readable path in the validation model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<BoundedText>,
}

/// Indirect PDF object key.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectKey {
    /// Object number.
    pub number: NonZeroU32,
    /// Generation number.
    pub generation: u16,
}

impl ObjectKey {
    /// Creates an indirect object key.
    #[must_use]
    pub fn new(number: NonZeroU32, generation: u16) -> Self {
        Self { number, generation }
    }
}

/// Error template argument.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorArgument {
    /// Argument name.
    pub name: Identifier,
    /// Argument value.
    pub value: BoundedText,
}

/// Unsupported rule detail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsupportedRule {
    /// Profile id that owns the rule.
    pub profile_id: Identifier,
    /// Unsupported rule id.
    pub rule_id: RuleId,
    /// Expression fragment when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_fragment: Option<BoundedText>,
    /// Unsupported reason.
    pub reason: BoundedText,
    /// Specification citations associated with this rule.
    pub references: Vec<SpecReference>,
}

/// Specification citation associated with a validation rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecReference {
    /// Specification name.
    pub specification: BoundedText,
    /// Clause or section identifier.
    pub clause: BoundedText,
}

/// Parser fact emitted by tolerant parsing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ParseFact {
    /// Header fact.
    Header {
        /// Header offset in bytes.
        offset: u64,
        /// PDF version.
        version: PdfVersion,
        /// Whether bytes preceded the header.
        #[serde(rename = "hadLeadingBytes")]
        had_leading_bytes: bool,
    },
    /// Bytes after EOF marker.
    PostEofData {
        /// Post-EOF byte count.
        bytes: u64,
    },
    /// Cross-reference fact.
    Xref {
        /// Xref section location.
        section: ObjectLocation,
        /// Xref-specific fact.
        fact: XrefFact,
    },
    /// Stream fact.
    Stream {
        /// Stream object key.
        object: ObjectKey,
        /// Stream-specific fact.
        fact: StreamFact,
    },
    /// Encryption fact.
    Encryption {
        /// Whether encryption was detected.
        encrypted: bool,
        /// Encryption handler when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        handler: Option<Identifier>,
        /// Encryption version when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<u8>,
        /// Security handler revision when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        revision: Option<u8>,
        /// Selected object encryption algorithm when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        algorithm: Option<Identifier>,
        /// Whether decryption succeeded.
        decrypted: bool,
    },
    /// XMP metadata fact.
    Xmp {
        /// Metadata stream object key.
        object: ObjectKey,
        /// XMP-specific fact.
        fact: XmpFact,
    },
}

/// PDF version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfVersion {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
}

/// Cross-reference parser fact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum XrefFact {
    /// Classic xref section had compliant EOL markers.
    EolMarkersComply,
    /// Classic xref section was malformed but recoverable.
    MalformedClassic,
    /// Xref stream was detected and is unsupported in M0.
    XrefStreamUnsupported,
    /// Xref stream was parsed.
    XrefStreamParsed {
        /// Number of xref entries parsed.
        entries: u64,
        /// Number of compressed-object entries parsed.
        compressed_entries: u64,
    },
    /// A previous xref section offset was declared.
    PrevChain {
        /// Previous xref byte offset.
        offset: u64,
    },
    /// A hybrid-reference xref stream offset was declared.
    HybridReference {
        /// Hybrid xref stream byte offset.
        offset: u64,
    },
    /// Object stream was parsed and expanded.
    ObjectStreamParsed,
}

/// Stream parser fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "fact")]
pub enum StreamFact {
    /// Declared and discovered stream lengths.
    Length {
        /// Declared stream length.
        declared: u64,
        /// Discovered stream length.
        discovered: u64,
    },
    /// Stream keyword spacing compliance.
    KeywordSpacing {
        /// `stream` keyword spacing compliance.
        #[serde(rename = "streamKeywordCRLFCompliant")]
        stream_keyword_crlf_compliant: bool,
        /// `endstream` keyword spacing compliance.
        #[serde(rename = "endstreamKeywordEolCompliant")]
        endstream_keyword_eol_compliant: bool,
    },
    /// Stream was decoded within configured limits.
    Decoded {
        /// Decoded stream byte count.
        bytes: u64,
    },
    /// A single stream filter decoded successfully.
    FilterDecoded {
        /// Filter name.
        filter: Identifier,
        /// Input bytes consumed by this filter.
        input_bytes: u64,
        /// Output bytes produced by this filter.
        output_bytes: u64,
    },
    /// A filter was retained in byte-preserving metadata mode.
    FilterMetadataMode {
        /// Filter name.
        filter: Identifier,
        /// Bytes preserved without pixel/image decoding.
        bytes: u64,
    },
}

/// XMP metadata parser fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "fact")]
pub enum XmpFact {
    /// XMP packet was extracted and parsed.
    PacketParsed {
        /// Packet byte count.
        bytes: u64,
        /// Number of namespace declarations retained.
        namespaces: u64,
        /// Number of recognized flavour claims.
        claims: u64,
    },
    /// XMP packet wrapper was absent.
    MissingPacketWrapper,
    /// Recognized XMP flavour claim.
    FlavourClaim {
        /// Flavour family.
        family: Identifier,
        /// Profile display spelling.
        display_flavour: BoundedText,
        /// Namespace URI that supplied the claim.
        namespace_uri: BoundedText,
    },
    /// XMP XML was malformed or unsupported.
    Malformed {
        /// Bounded reason string.
        reason: BoundedText,
    },
    /// DTD or entity processing was rejected.
    HostileXmlRejected {
        /// Bounded reason string.
        reason: BoundedText,
    },
}

/// Validation warning.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ValidationWarning {
    /// Parse facts exceeded the configured retention cap.
    ParseFactCapReached {
        /// Configured cap.
        cap: usize,
    },
    /// Incompatible profile was skipped.
    IncompatibleProfile {
        /// Profile id.
        profile_id: Identifier,
        /// Skip reason.
        reason: BoundedText,
    },
    /// Auto flavour detection fell back or could not select a profile.
    AutoDetection {
        /// Bounded warning message.
        message: BoundedText,
    },
    /// General bounded warning.
    General {
        /// Warning message.
        message: BoundedText,
    },
}

impl ValidationWarning {
    /// Returns a bounded human-readable warning message.
    #[must_use]
    pub fn message_text(&self) -> BoundedText {
        match self {
            Self::ParseFactCapReached { cap } => {
                BoundedText::unchecked(format!("parse fact cap reached: {cap}"))
            }
            Self::IncompatibleProfile { profile_id, reason } => BoundedText::unchecked(format!(
                "incompatible profile {}: {}",
                profile_id.as_str(),
                reason.as_str()
            )),
            Self::AutoDetection { message } => {
                BoundedText::unchecked(format!("auto detection: {}", message.as_str()))
            }
            Self::General { message } => message.clone(),
        }
    }
}

/// Task duration entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskDuration {
    /// Task name.
    pub task: Identifier,
    /// Duration in milliseconds.
    pub millis: u64,
}

impl TaskDuration {
    /// Creates a task duration from a [`Duration`].
    ///
    /// Values larger than `u64::MAX` milliseconds saturate.
    #[must_use]
    pub fn from_duration(task: Identifier, duration: Duration) -> Self {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        Self { task, millis }
    }
}

/// Batch validation report.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchReport {
    /// Item reports.
    pub items: Vec<ValidationReport>,
    /// Batch summary.
    pub summary: BatchSummary,
    /// Batch-level warnings.
    pub warnings: Vec<ValidationWarning>,
}

impl BatchReport {
    /// Builds a batch report and computes summary counters from item reports.
    #[must_use]
    pub fn from_items(
        items: Vec<ValidationReport>,
        warnings: Vec<ValidationWarning>,
        elapsed: Duration,
    ) -> Self {
        let summary = BatchSummary::from_items(&items, elapsed);
        Self {
            items,
            summary,
            warnings,
        }
    }

    /// Builds a batch report with internal per-input error count.
    #[must_use]
    pub fn from_items_with_internal_errors(
        items: Vec<ValidationReport>,
        warnings: Vec<ValidationWarning>,
        elapsed: Duration,
        internal_errors: u64,
    ) -> Self {
        let summary =
            BatchSummary::from_items_with_internal_errors(&items, elapsed, internal_errors);
        Self {
            items,
            summary,
            warnings,
        }
    }
}

/// Batch summary counters.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TypedBuilder)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchSummary {
    /// Total input count.
    pub total_files: u64,
    /// Valid input count.
    pub valid: u64,
    /// Invalid input count.
    pub invalid: u64,
    /// Parse failure count.
    pub parse_failures: u64,
    /// Encrypted input count.
    pub encrypted: u64,
    /// Incomplete validation count.
    pub incomplete: u64,
    /// Internal error count.
    pub internal_errors: u64,
    /// Elapsed milliseconds.
    pub elapsed_millis: u64,
    /// Worst exit category.
    pub worst_exit_category: ExitCategory,
}

impl BatchSummary {
    /// Computes batch summary counters from item reports.
    #[must_use]
    pub fn from_items(items: &[ValidationReport], elapsed: Duration) -> Self {
        let mut summary = Self {
            total_files: u64::try_from(items.len()).unwrap_or(u64::MAX),
            elapsed_millis: duration_millis(elapsed),
            ..Self::default()
        };
        summary.apply_items(items);
        summary.finish()
    }

    /// Computes batch summary counters from item reports plus internal error count.
    #[must_use]
    pub fn from_items_with_internal_errors(
        items: &[ValidationReport],
        elapsed: Duration,
        internal_errors: u64,
    ) -> Self {
        let mut summary = Self {
            total_files: u64::try_from(items.len())
                .unwrap_or(u64::MAX)
                .saturating_add(internal_errors),
            elapsed_millis: duration_millis(elapsed),
            internal_errors,
            ..Self::default()
        };
        summary.apply_items(items);
        summary.finish()
    }

    fn apply_items(&mut self, items: &[ValidationReport]) {
        for report in items {
            match report.status {
                ValidationStatus::Valid => self.valid = self.valid.saturating_add(1),
                ValidationStatus::Invalid => self.invalid = self.invalid.saturating_add(1),
                ValidationStatus::ParseFailed => {
                    self.parse_failures = self.parse_failures.saturating_add(1);
                }
                ValidationStatus::Encrypted => {
                    self.encrypted = self.encrypted.saturating_add(1);
                }
                ValidationStatus::Incomplete => {
                    self.incomplete = self.incomplete.saturating_add(1);
                }
            }
        }
    }

    fn finish(mut self) -> Self {
        self.worst_exit_category = if self.parse_failures > 0
            || self.encrypted > 0
            || self.incomplete > 0
            || self.internal_errors > 0
        {
            ExitCategory::ProcessingFailed
        } else if self.invalid > 0 {
            ExitCategory::ValidationFailed
        } else {
            ExitCategory::Success
        };
        self
    }
}

/// CLI-oriented exit category represented in batch summaries.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ExitCategory {
    /// Success.
    #[default]
    Success,
    /// Validation found non-compliance.
    ValidationFailed,
    /// Input could not be processed.
    ProcessingFailed,
    /// Internal application failure.
    InternalError,
}

/// Report output format.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ReportFormat {
    /// Compact JSON.
    Json,
    /// Pretty-printed JSON.
    JsonPretty,
    /// Human-readable text.
    Text,
    /// Machine-readable XML compatibility report.
    Xml,
    /// Raw processor-style XML report.
    RawXml,
    /// Static human-readable HTML report.
    Html,
}

impl ReportFormat {
    /// Writes a validation report in this format.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    pub fn write_report<W: Write>(&self, report: &ValidationReport, out: W) -> Result<()> {
        match self {
            Self::Json => JsonReportWriter::compact().write_report(report, out),
            Self::JsonPretty => JsonReportWriter::pretty().write_report(report, out),
            Self::Text => TextReportWriter.write_report(report, out),
            Self::Xml => XmlReportWriter.write_report(report, out),
            Self::RawXml => RawXmlReportWriter.write_report(report, out),
            Self::Html => HtmlReportWriter.write_report(report, out),
        }
    }

    /// Writes a batch validation report in this format.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    pub fn write_batch<W: Write>(&self, report: &BatchReport, out: W) -> Result<()> {
        match self {
            Self::Json => JsonReportWriter::compact().write_batch(report, out),
            Self::JsonPretty => JsonReportWriter::pretty().write_batch(report, out),
            Self::Text => TextReportWriter.write_batch(report, out),
            Self::Xml => XmlReportWriter.write_batch(report, out),
            Self::RawXml => RawXmlReportWriter.write_batch(report, out),
            Self::Html => HtmlReportWriter.write_batch(report, out),
        }
    }

    /// Writes a metadata repair report in this format.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    pub fn write_repair_report<W: Write>(&self, report: &RepairReport, out: W) -> Result<()> {
        match self {
            Self::Json => JsonReportWriter::compact().write_repair_report(report, out),
            Self::JsonPretty => JsonReportWriter::pretty().write_repair_report(report, out),
            Self::Text => TextReportWriter.write_repair_report(report, out),
            Self::Xml => XmlReportWriter.write_repair_report(report, out),
            Self::RawXml => RawXmlReportWriter.write_repair_report(report, out),
            Self::Html => HtmlReportWriter.write_repair_report(report, out),
        }
    }

    /// Writes a batch metadata repair report in this format.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    pub fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, out: W) -> Result<()> {
        match self {
            Self::Json => JsonReportWriter::compact().write_repair_batch(report, out),
            Self::JsonPretty => JsonReportWriter::pretty().write_repair_batch(report, out),
            Self::Text => TextReportWriter.write_repair_batch(report, out),
            Self::Xml => XmlReportWriter.write_repair_batch(report, out),
            Self::RawXml => RawXmlReportWriter.write_repair_batch(report, out),
            Self::Html => HtmlReportWriter.write_repair_batch(report, out),
        }
    }
}

/// Report writer interface.
pub trait ReportWriter {
    /// Writes a single validation report.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    fn write_report<W: Write>(&self, report: &ValidationReport, out: W) -> Result<()>;

    /// Writes a batch validation report.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    fn write_batch<W: Write>(&self, report: &BatchReport, out: W) -> Result<()>;

    /// Writes a single metadata repair report.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    fn write_repair_report<W: Write>(&self, report: &RepairReport, out: W) -> Result<()>;

    /// Writes a batch metadata repair report.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] if serialization or writing fails.
    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, out: W) -> Result<()>;
}

/// JSON report writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonReportWriter {
    pretty: bool,
}

impl JsonReportWriter {
    /// Creates a compact JSON report writer.
    #[must_use]
    pub fn compact() -> Self {
        Self { pretty: false }
    }

    /// Creates a pretty JSON report writer.
    #[must_use]
    pub fn pretty() -> Self {
        Self { pretty: true }
    }
}

impl ReportWriter for JsonReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, out: W) -> Result<()> {
        write_json(out, report, self.pretty)
    }

    fn write_batch<W: Write>(&self, report: &BatchReport, out: W) -> Result<()> {
        write_json(out, report, self.pretty)
    }

    fn write_repair_report<W: Write>(&self, report: &RepairReport, out: W) -> Result<()> {
        write_json(out, report, self.pretty)
    }

    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, out: W) -> Result<()> {
        write_json(out, report, self.pretty)
    }
}

/// Human-readable text report writer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextReportWriter;

impl ReportWriter for TextReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, mut out: W) -> Result<()> {
        write_text_report(report, &mut out)
    }

    fn write_batch<W: Write>(&self, report: &BatchReport, mut out: W) -> Result<()> {
        writeln!(
            out,
            "batch: {}",
            exit_category_text(report.summary.worst_exit_category)
        )
        .map_err(write_error)?;
        writeln!(out, "files: {}", report.summary.total_files).map_err(write_error)?;
        writeln!(
            out,
            "summary: {} valid, {} invalid, {} parse failed, {} encrypted, {} incomplete, {} \
             internal errors",
            report.summary.valid,
            report.summary.invalid,
            report.summary.parse_failures,
            report.summary.encrypted,
            report.summary.incomplete,
            report.summary.internal_errors,
        )
        .map_err(write_error)?;
        if !report.warnings.is_empty() {
            writeln!(out, "warnings: {}", report.warnings.len()).map_err(write_error)?;
        }
        writeln!(out, "items:").map_err(write_error)?;
        for item in &report.items {
            writeln!(
                out,
                "  {}: {}",
                source_name(&item.source),
                status_text(item.status)
            )
            .map_err(write_error)?;
        }
        Ok(())
    }

    fn write_repair_report<W: Write>(&self, report: &RepairReport, mut out: W) -> Result<()> {
        write_text_repair_report(report, &mut out)
    }

    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, mut out: W) -> Result<()> {
        writeln!(
            out,
            "repair batch: {}",
            exit_category_text(report.summary.worst_exit_category)
        )
        .map_err(write_error)?;
        writeln!(out, "files: {}", report.summary.total_files).map_err(write_error)?;
        writeln!(
            out,
            "summary: {} repaired, {} unchanged, {} refused, {} failed",
            report.summary.succeeded,
            report.summary.no_action,
            report.summary.refused,
            report.summary.failed,
        )
        .map_err(write_error)?;
        writeln!(out, "items:").map_err(write_error)?;
        for item in &report.items {
            writeln!(
                out,
                "  {}: {}",
                source_name(&item.source),
                repair_status_text(item.status),
            )
            .map_err(write_error)?;
        }
        Ok(())
    }
}

/// Machine-readable XML report writer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct XmlReportWriter;

impl ReportWriter for XmlReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, mut out: W) -> Result<()> {
        let batch = BatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_xml_batch(&batch, &mut out)
    }

    fn write_batch<W: Write>(&self, report: &BatchReport, mut out: W) -> Result<()> {
        write_xml_batch(report, &mut out)
    }

    fn write_repair_report<W: Write>(&self, report: &RepairReport, mut out: W) -> Result<()> {
        let batch = RepairBatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_xml_repair_batch(&batch, &mut out, "repairReport")
    }

    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, mut out: W) -> Result<()> {
        write_xml_repair_batch(report, &mut out, "repairReport")
    }
}

/// Raw processor-style XML report writer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RawXmlReportWriter;

impl ReportWriter for RawXmlReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, mut out: W) -> Result<()> {
        let batch = BatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_raw_xml_batch(&batch, &mut out)
    }

    fn write_batch<W: Write>(&self, report: &BatchReport, mut out: W) -> Result<()> {
        write_raw_xml_batch(report, &mut out)
    }

    fn write_repair_report<W: Write>(&self, report: &RepairReport, mut out: W) -> Result<()> {
        let batch = RepairBatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_xml_repair_batch(&batch, &mut out, "rawRepairReport")
    }

    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, mut out: W) -> Result<()> {
        write_xml_repair_batch(report, &mut out, "rawRepairReport")
    }
}

/// Static HTML report writer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HtmlReportWriter;

impl ReportWriter for HtmlReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, mut out: W) -> Result<()> {
        let batch = BatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_html_batch(&batch, &mut out)
    }

    fn write_batch<W: Write>(&self, report: &BatchReport, mut out: W) -> Result<()> {
        write_html_batch(report, &mut out)
    }

    fn write_repair_report<W: Write>(&self, report: &RepairReport, mut out: W) -> Result<()> {
        let batch = RepairBatchReport::from_items(vec![report.clone()], Vec::new(), Duration::ZERO);
        write_html_repair_batch(&batch, &mut out)
    }

    fn write_repair_batch<W: Write>(&self, report: &RepairBatchReport, mut out: W) -> Result<()> {
        write_html_repair_batch(report, &mut out)
    }
}

fn write_json<W, T>(out: W, value: &T, pretty: bool) -> Result<()>
where
    W: Write,
    T: Serialize,
{
    if pretty {
        serde_json::to_writer_pretty(out, value).map_err(ReportError::from)?;
    } else {
        serde_json::to_writer(out, value).map_err(ReportError::from)?;
    }
    Ok(())
}

fn write_text_report<W: Write>(report: &ValidationReport, out: &mut W) -> Result<()> {
    writeln!(
        out,
        "{}: {}",
        source_name(&report.source),
        status_text(report.status),
    )
    .map_err(write_error)?;
    writeln!(out, "profiles: {}", profile_list(report)).map_err(write_error)?;
    let checks = check_counts(report);
    writeln!(
        out,
        "checks: {} passed, {} failed, {} unsupported",
        checks.passed, checks.failed, checks.unsupported,
    )
    .map_err(write_error)?;
    let failures = report
        .profile_reports
        .iter()
        .flat_map(|profile| profile.failed_assertions.iter())
        .take(5)
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        writeln!(out, "first failures:").map_err(write_error)?;
        for assertion in failures {
            writeln!(
                out,
                "  {} at {}: {}",
                assertion.rule_id.0.as_str(),
                location_text(&assertion.location),
                assertion_message(assertion),
            )
            .map_err(write_error)?;
        }
    }
    let unsupported = report
        .profile_reports
        .iter()
        .flat_map(|profile| profile.unsupported_rules.iter())
        .take(5)
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        writeln!(out, "unsupported rules:").map_err(write_error)?;
        for rule in unsupported {
            writeln!(
                out,
                "  {}: {}{}",
                rule.rule_id.0.as_str(),
                rule.reason.as_str(),
                reference_suffix(&rule.references),
            )
            .map_err(write_error)?;
        }
    }
    if !report.warnings.is_empty() {
        writeln!(out, "warnings: {}", report.warnings.len()).map_err(write_error)?;
    }
    if let Some(features) = &report.feature_report {
        writeln!(out, "features: {} objects", features.objects.len()).map_err(write_error)?;
    }
    if let Some(policy) = &report.policy_report {
        writeln!(
            out,
            "policy: {}",
            if policy.is_compliant {
                "compliant"
            } else {
                "non-compliant"
            }
        )
        .map_err(write_error)?;
    }
    Ok(())
}

fn write_text_repair_report<W: Write>(report: &RepairReport, out: &mut W) -> Result<()> {
    writeln!(
        out,
        "{}: {}",
        source_name(&report.source),
        repair_status_text(report.status),
    )
    .map_err(write_error)?;
    if let Some(output_path) = &report.output_path {
        writeln!(out, "output: {}", output_path.display()).map_err(write_error)?;
    }
    if !report.actions.is_empty() {
        writeln!(out, "actions: {}", report.actions.len()).map_err(write_error)?;
        for action in &report.actions {
            writeln!(out, "  {}", repair_action_text(action)).map_err(write_error)?;
        }
    }
    if let Some(refusal) = &report.refusal {
        writeln!(out, "refusal: {}", repair_refusal_text(refusal)).map_err(write_error)?;
    }
    if !report.warnings.is_empty() {
        writeln!(out, "warnings: {}", report.warnings.len()).map_err(write_error)?;
    }
    Ok(())
}

fn write_xml_batch<W: Write>(report: &BatchReport, out: &mut W) -> Result<()> {
    writeln!(out, r#"<?xml version="1.0" encoding="utf-8"?>"#).map_err(write_error)?;
    writeln!(out, "<report>").map_err(write_error)?;
    writeln!(out, "  <buildInformation>").map_err(write_error)?;
    writeln!(
        out,
        r#"    <releaseDetails id="pdfv-core" version="{}"></releaseDetails>"#,
        XmlEscapedAttr::new(ENGINE_VERSION)?,
    )
    .map_err(write_error)?;
    writeln!(out, "  </buildInformation>").map_err(write_error)?;
    writeln!(out, "  <jobs>").map_err(write_error)?;
    for item in &report.items {
        write_xml_job(item, out)?;
    }
    writeln!(out, "  </jobs>").map_err(write_error)?;
    write_xml_batch_summary(&report.summary, out)?;
    write_xml_warnings(&report.warnings, out, 2)?;
    writeln!(out, "</report>").map_err(write_error)?;
    Ok(())
}

fn write_raw_xml_batch<W: Write>(report: &BatchReport, out: &mut W) -> Result<()> {
    writeln!(out, r#"<?xml version="1.0" encoding="utf-8"?>"#).map_err(write_error)?;
    writeln!(
        out,
        r#"<rawReport engine="pdfv-core" version="{}">"#,
        XmlEscapedAttr::new(ENGINE_VERSION)?,
    )
    .map_err(write_error)?;
    writeln!(
        out,
        r#"  <processorConfig tasks="{}"></processorConfig>"#,
        XmlEscapedAttr::new(&raw_validation_tasks(report))?,
    )
    .map_err(write_error)?;
    writeln!(out, "  <processorResults>").map_err(write_error)?;
    for item in &report.items {
        writeln!(
            out,
            r#"    <processorResult status="{}">"#,
            status_text(item.status),
        )
        .map_err(write_error)?;
        write_xml_item(&item.source, out)?;
        for profile in &item.profile_reports {
            write_xml_validation_report(item.status, profile, out)?;
        }
        if let Some(feature_report) = &item.feature_report {
            write_xml_feature_report(feature_report, out)?;
        }
        if let Some(policy_report) = &item.policy_report {
            write_xml_policy_report(policy_report, out)?;
        }
        write_xml_parse_facts(&item.parse_facts, out)?;
        write_xml_warnings(&item.warnings, out, 6)?;
        writeln!(out, "    </processorResult>").map_err(write_error)?;
    }
    writeln!(out, "  </processorResults>").map_err(write_error)?;
    write_xml_batch_summary(&report.summary, out)?;
    writeln!(out, "</rawReport>").map_err(write_error)?;
    Ok(())
}

fn write_xml_repair_batch<W: Write>(
    report: &RepairBatchReport,
    out: &mut W,
    root: &str,
) -> Result<()> {
    writeln!(out, r#"<?xml version="1.0" encoding="utf-8"?>"#).map_err(write_error)?;
    writeln!(
        out,
        r#"<{root} engine="pdfv-core" version="{}">"#,
        XmlEscapedAttr::new(ENGINE_VERSION)?,
    )
    .map_err(write_error)?;
    if root == "rawRepairReport" {
        writeln!(
            out,
            r#"  <processorConfig tasks="metadata"></processorConfig>"#,
        )
        .map_err(write_error)?;
    }
    writeln!(out, "  <items>").map_err(write_error)?;
    for item in &report.items {
        write_xml_repair_item(item, out)?;
    }
    writeln!(out, "  </items>").map_err(write_error)?;
    write_xml_repair_summary(&report.summary, out)?;
    write_xml_warnings(&report.warnings, out, 2)?;
    writeln!(out, "</{root}>").map_err(write_error)?;
    Ok(())
}

fn write_xml_repair_item<W: Write>(report: &RepairReport, out: &mut W) -> Result<()> {
    writeln!(
        out,
        r#"    <repairItem status="{}">"#,
        repair_status_text(report.status),
    )
    .map_err(write_error)?;
    write_xml_item(&report.source, out)?;
    if let Some(output_path) = &report.output_path {
        writeln!(
            out,
            "      <output>{}</output>",
            XmlEscapedText::new(&output_path.display().to_string())?,
        )
        .map_err(write_error)?;
    }
    if !report.actions.is_empty() {
        writeln!(out, "      <actions>").map_err(write_error)?;
        for action in &report.actions {
            writeln!(
                out,
                r#"        <action kind="{}">{}</action>"#,
                repair_action_kind(action),
                XmlEscapedText::new(&repair_action_text(action))?,
            )
            .map_err(write_error)?;
        }
        writeln!(out, "      </actions>").map_err(write_error)?;
    }
    if let Some(refusal) = &report.refusal {
        writeln!(
            out,
            r#"      <refusal kind="{}">{}</refusal>"#,
            repair_refusal_kind(refusal),
            XmlEscapedText::new(&repair_refusal_text(refusal))?,
        )
        .map_err(write_error)?;
    }
    write_xml_warnings(&report.warnings, out, 6)?;
    writeln!(out, "    </repairItem>").map_err(write_error)?;
    Ok(())
}

fn write_xml_repair_summary<W: Write>(summary: &RepairBatchSummary, out: &mut W) -> Result<()> {
    writeln!(
        out,
        r#"  <repairSummary totalJobs="{}" succeeded="{}" noAction="{}" refused="{}" failed="{}" elapsedMillis="{}"></repairSummary>"#,
        summary.total_files,
        summary.succeeded,
        summary.no_action,
        summary.refused,
        summary.failed,
        summary.elapsed_millis,
    )
    .map_err(write_error)?;
    Ok(())
}

fn raw_validation_tasks(report: &BatchReport) -> String {
    let has_features = report
        .items
        .iter()
        .any(|item| item.feature_report.is_some());
    let has_policy = report.items.iter().any(|item| item.policy_report.is_some());
    let mut tasks = vec!["validation"];
    if has_features {
        tasks.push("features");
    }
    if has_policy {
        tasks.push("policy");
    }
    tasks.join(",")
}

fn write_html_batch<W: Write>(report: &BatchReport, out: &mut W) -> Result<()> {
    write_html_start(out, "pdfv validation report")?;
    writeln!(out, "<h1>Validation Report</h1>").map_err(write_error)?;
    writeln!(
        out,
        "<p>{} valid, {} invalid, {} parse failed, {} encrypted, {} incomplete.</p>",
        report.summary.valid,
        report.summary.invalid,
        report.summary.parse_failures,
        report.summary.encrypted,
        report.summary.incomplete,
    )
    .map_err(write_error)?;
    writeln!(
        out,
        "<table><thead><tr><th>Input</th><th>Status</th><th>Profiles</th><th>Features</\
         th><th>Policy</th></tr></thead><tbody>"
    )
    .map_err(write_error)?;
    for item in &report.items {
        let features = item
            .feature_report
            .as_ref()
            .map_or(String::from("-"), |features| {
                features.objects.len().to_string()
            });
        let policy = item.policy_report.as_ref().map_or("-", |policy| {
            if policy.is_compliant {
                "compliant"
            } else {
                "non-compliant"
            }
        });
        writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            HtmlEscapedText::new(&source_name(&item.source))?,
            status_text(item.status),
            HtmlEscapedText::new(&profile_list(item))?,
            features,
            policy,
        )
        .map_err(write_error)?;
    }
    writeln!(out, "</tbody></table>").map_err(write_error)?;
    write_html_end(out)
}

fn write_html_repair_batch<W: Write>(report: &RepairBatchReport, out: &mut W) -> Result<()> {
    write_html_start(out, "pdfv metadata repair report")?;
    writeln!(out, "<h1>Metadata Repair Report</h1>").map_err(write_error)?;
    writeln!(
        out,
        "<p>{} repaired, {} unchanged, {} refused, {} failed.</p>",
        report.summary.succeeded,
        report.summary.no_action,
        report.summary.refused,
        report.summary.failed,
    )
    .map_err(write_error)?;
    writeln!(
        out,
        "<table><thead><tr><th>Input</th><th>Status</th><th>Output</th><th>Reason</th></tr></\
         thead><tbody>"
    )
    .map_err(write_error)?;
    for item in &report.items {
        let output = item
            .output_path
            .as_ref()
            .map_or_else(String::new, |path| path.display().to_string());
        let reason = item
            .refusal
            .as_ref()
            .map_or_else(String::new, repair_refusal_text);
        writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            HtmlEscapedText::new(&source_name(&item.source))?,
            repair_status_text(item.status),
            HtmlEscapedText::new(&output)?,
            HtmlEscapedText::new(&reason)?,
        )
        .map_err(write_error)?;
    }
    writeln!(out, "</tbody></table>").map_err(write_error)?;
    write_html_end(out)
}

fn write_html_start<W: Write>(out: &mut W, title: &str) -> Result<()> {
    writeln!(out, "<!doctype html>").map_err(write_error)?;
    writeln!(
        out,
        r#"<html lang="en"><head><meta charset="utf-8"><title>{}</title><style>body{{font-family:system-ui,sans-serif;margin:2rem;color:#1f2937}}table{{border-collapse:collapse;width:100%}}th,td{{border:1px solid #d1d5db;padding:.4rem;text-align:left}}th{{background:#f3f4f6}}</style></head><body>"#,
        HtmlEscapedText::new(title)?,
    )
    .map_err(write_error)?;
    Ok(())
}

fn write_html_end<W: Write>(out: &mut W) -> Result<()> {
    writeln!(out, "</body></html>").map_err(write_error)?;
    Ok(())
}

fn write_xml_job<W: Write>(report: &ValidationReport, out: &mut W) -> Result<()> {
    writeln!(out, "    <job>").map_err(write_error)?;
    write_xml_item(&report.source, out)?;
    for profile in &report.profile_reports {
        write_xml_validation_report(report.status, profile, out)?;
    }
    if report.profile_reports.is_empty() {
        writeln!(
            out,
            r#"      <validationReport profileName="" statement="{}" isCompliant="false">"#,
            XmlEscapedAttr::new(status_statement(report.status))?,
        )
        .map_err(write_error)?;
        writeln!(
            out,
            r#"        <details passedRules="0" failedRules="0" passedChecks="0" failedChecks="0" unsupportedRules="0"></details>"#,
        )
        .map_err(write_error)?;
        writeln!(out, "      </validationReport>").map_err(write_error)?;
    }
    write_xml_parse_facts(&report.parse_facts, out)?;
    if let Some(feature_report) = &report.feature_report {
        write_xml_feature_report(feature_report, out)?;
    }
    if let Some(policy_report) = &report.policy_report {
        write_xml_policy_report(policy_report, out)?;
    }
    write_xml_warnings(&report.warnings, out, 6)?;
    writeln!(out, "    </job>").map_err(write_error)?;
    Ok(())
}

fn write_xml_item<W: Write>(source: &InputSummary, out: &mut W) -> Result<()> {
    let size = source
        .bytes
        .map_or_else(String::new, |bytes| format!(r#" size="{bytes}""#));
    writeln!(out, "      <item{size}>").map_err(write_error)?;
    let name = source_name(source);
    writeln!(out, "        <name>{}</name>", XmlEscapedText::new(&name)?).map_err(write_error)?;
    writeln!(out, "      </item>").map_err(write_error)?;
    Ok(())
}

fn write_xml_validation_report<W: Write>(
    status: ValidationStatus,
    profile: &ProfileReport,
    out: &mut W,
) -> Result<()> {
    writeln!(
        out,
        r#"      <validationReport profileName="{}" statement="{}" isCompliant="{}">"#,
        XmlEscapedAttr::new(profile.profile.name.as_str())?,
        XmlEscapedAttr::new(status_statement(status))?,
        profile.is_compliant,
    )
    .map_err(write_error)?;
    let failed_checks = u64::try_from(profile.failed_assertions.len()).unwrap_or(u64::MAX);
    let unsupported_rules = u64::try_from(profile.unsupported_rules.len()).unwrap_or(u64::MAX);
    let passed_checks = profile.checks_executed.saturating_sub(failed_checks);
    let passed_rules = profile.rules_executed.saturating_sub(profile.failed_rules);
    writeln!(
        out,
        r#"        <details passedRules="{passed_rules}" failedRules="{}" passedChecks="{passed_checks}" failedChecks="{failed_checks}" unsupportedRules="{unsupported_rules}"></details>"#,
        profile.failed_rules,
    )
    .map_err(write_error)?;
    write_xml_assertions("failedChecks", &profile.failed_assertions, out)?;
    write_xml_assertions("passedChecks", &profile.passed_assertions, out)?;
    write_xml_unsupported_rules(&profile.unsupported_rules, out)?;
    writeln!(out, "      </validationReport>").map_err(write_error)?;
    Ok(())
}

fn write_xml_assertions<W: Write>(
    element: &str,
    assertions: &[Assertion],
    out: &mut W,
) -> Result<()> {
    if assertions.is_empty() {
        return Ok(());
    }
    writeln!(out, "        <{element}>").map_err(write_error)?;
    for assertion in assertions {
        writeln!(
            out,
            r#"          <check ruleId="{}" status="{}" location="{}">"#,
            XmlEscapedAttr::new(assertion.rule_id.0.as_str())?,
            assertion_status_text(assertion.status),
            XmlEscapedAttr::new(&location_text(&assertion.location))?,
        )
        .map_err(write_error)?;
        writeln!(
            out,
            "            <description>{}</description>",
            XmlEscapedText::new(assertion.description.as_str())?,
        )
        .map_err(write_error)?;
        if let Some(message) = &assertion.message {
            writeln!(
                out,
                "            <message>{}</message>",
                XmlEscapedText::new(message.as_str())?,
            )
            .map_err(write_error)?;
        }
        if !assertion.error_arguments.is_empty() {
            writeln!(out, "            <errorArguments>").map_err(write_error)?;
            for argument in &assertion.error_arguments {
                writeln!(
                    out,
                    r#"              <argument name="{}">{}</argument>"#,
                    XmlEscapedAttr::new(argument.name.as_str())?,
                    XmlEscapedText::new(argument.value.as_str())?,
                )
                .map_err(write_error)?;
            }
            writeln!(out, "            </errorArguments>").map_err(write_error)?;
        }
        writeln!(out, "          </check>").map_err(write_error)?;
    }
    writeln!(out, "        </{element}>").map_err(write_error)?;
    Ok(())
}

fn write_xml_unsupported_rules<W: Write>(rules: &[UnsupportedRule], out: &mut W) -> Result<()> {
    if rules.is_empty() {
        return Ok(());
    }
    writeln!(out, "        <unsupportedRules>").map_err(write_error)?;
    for rule in rules {
        writeln!(
            out,
            r#"          <rule profileId="{}" ruleId="{}">"#,
            XmlEscapedAttr::new(rule.profile_id.as_str())?,
            XmlEscapedAttr::new(rule.rule_id.0.as_str())?,
        )
        .map_err(write_error)?;
        if let Some(fragment) = &rule.expression_fragment {
            writeln!(
                out,
                "            <expression>{}</expression>",
                XmlEscapedText::new(fragment.as_str())?,
            )
            .map_err(write_error)?;
        }
        writeln!(
            out,
            "            <reason>{}</reason>",
            XmlEscapedText::new(rule.reason.as_str())?,
        )
        .map_err(write_error)?;
        if !rule.references.is_empty() {
            writeln!(out, "            <references>").map_err(write_error)?;
            for reference in &rule.references {
                writeln!(
                    out,
                    r#"              <reference specification="{}" clause="{}"></reference>"#,
                    XmlEscapedAttr::new(reference.specification.as_str())?,
                    XmlEscapedAttr::new(reference.clause.as_str())?,
                )
                .map_err(write_error)?;
            }
            writeln!(out, "            </references>").map_err(write_error)?;
        }
        writeln!(out, "          </rule>").map_err(write_error)?;
    }
    writeln!(out, "        </unsupportedRules>").map_err(write_error)?;
    Ok(())
}

fn write_xml_feature_report<W: Write>(report: &FeatureReport, out: &mut W) -> Result<()> {
    writeln!(
        out,
        r#"      <featureReport visitedObjects="{}" extractedObjects="{}" truncated="{}">"#,
        report.visited_objects,
        report.objects.len(),
        report.truncated,
    )
    .map_err(write_error)?;
    for object in &report.objects {
        writeln!(
            out,
            r#"        <featureObject family="{}" location="{}">"#,
            XmlEscapedAttr::new(object.family.as_str())?,
            XmlEscapedAttr::new(&location_text(&object.location))?,
        )
        .map_err(write_error)?;
        for (name, value) in &object.properties {
            writeln!(
                out,
                r#"          <property name="{}">"#,
                XmlEscapedAttr::new(name.as_str())?,
            )
            .map_err(write_error)?;
            write_xml_feature_value(value, out, 12)?;
            writeln!(out, "          </property>").map_err(write_error)?;
        }
        writeln!(out, "        </featureObject>").map_err(write_error)?;
    }
    writeln!(out, "      </featureReport>").map_err(write_error)?;
    Ok(())
}

fn write_xml_policy_report<W: Write>(report: &PolicyReport, out: &mut W) -> Result<()> {
    writeln!(
        out,
        r#"      <policyReport name="{}" isCompliant="{}">"#,
        XmlEscapedAttr::new(report.name.as_ref().map_or("", BoundedText::as_str))?,
        report.is_compliant,
    )
    .map_err(write_error)?;
    for result in &report.results {
        writeln!(
            out,
            r#"        <rule id="{}" passed="{}" matches="{}">"#,
            XmlEscapedAttr::new(result.id.as_str())?,
            result.passed,
            result.matches,
        )
        .map_err(write_error)?;
        writeln!(
            out,
            "          <description>{}</description>",
            XmlEscapedText::new(result.description.as_str())?,
        )
        .map_err(write_error)?;
        writeln!(
            out,
            "          <message>{}</message>",
            XmlEscapedText::new(result.message.as_str())?,
        )
        .map_err(write_error)?;
        writeln!(out, "        </rule>").map_err(write_error)?;
    }
    writeln!(out, "      </policyReport>").map_err(write_error)?;
    Ok(())
}

fn reference_suffix(references: &[SpecReference]) -> String {
    let Some(reference) = references.first() else {
        return String::new();
    };
    format!(
        " [{} {}]",
        reference.specification.as_str(),
        reference.clause.as_str()
    )
}

fn write_xml_feature_value<W: Write>(
    value: &FeatureValue,
    out: &mut W,
    indent: usize,
) -> Result<()> {
    let spaces = " ".repeat(indent);
    match value {
        FeatureValue::Null => writeln!(out, r#"{spaces}<value type="null"></value>"#),
        FeatureValue::Bool(value) => {
            writeln!(out, r#"{spaces}<value type="bool">{value}</value>"#)
        }
        FeatureValue::Number(value) => {
            writeln!(out, r#"{spaces}<value type="number">{value}</value>"#)
        }
        FeatureValue::String(value) => writeln!(
            out,
            r#"{spaces}<value type="string">{}</value>"#,
            XmlEscapedText::new(value.as_str())?,
        ),
        FeatureValue::RedactedString { bytes } => writeln!(
            out,
            r#"{spaces}<value type="redactedString" bytes="{bytes}"></value>"#
        ),
        FeatureValue::ObjectKey(value) => writeln!(
            out,
            r#"{spaces}<value type="objectKey" number="{}" generation="{}"></value>"#,
            value.number, value.generation,
        ),
        FeatureValue::List(values) => {
            writeln!(out, r#"{spaces}<value type="list">"#).map_err(write_error)?;
            for item in values {
                write_xml_feature_value(item, out, indent.saturating_add(2))?;
            }
            writeln!(out, "{spaces}</value>")
        }
    }
    .map_err(write_error)?;
    Ok(())
}

fn write_xml_parse_facts<W: Write>(facts: &[ParseFact], out: &mut W) -> Result<()> {
    if facts.is_empty() {
        return Ok(());
    }
    writeln!(out, "      <parseFacts>").map_err(write_error)?;
    for fact in facts {
        match fact {
            ParseFact::Header {
                offset,
                version,
                had_leading_bytes,
            } => writeln!(
                out,
                r#"        <header offset="{offset}" version="{}.{}" hadLeadingBytes="{had_leading_bytes}"></header>"#,
                version.major,
                version.minor,
            )
            .map_err(write_error)?,
            ParseFact::PostEofData { bytes } => {
                writeln!(out, r#"        <postEofData bytes="{bytes}"></postEofData>"#)
                    .map_err(write_error)?;
            }
            ParseFact::Xref { section, fact } => writeln!(
                out,
                r#"        <xref location="{}" fact="{}"></xref>"#,
                XmlEscapedAttr::new(&location_text(section))?,
                XmlEscapedAttr::new(&xref_fact_text(fact))?,
            )
            .map_err(write_error)?,
            ParseFact::Stream { object, fact } => writeln!(
                out,
                r#"        <stream object="{} {}" fact="{}"></stream>"#,
                object.number,
                object.generation,
                XmlEscapedAttr::new(&stream_fact_text(fact))?,
            )
            .map_err(write_error)?,
            ParseFact::Encryption {
                encrypted,
                handler,
                version,
                revision,
                algorithm,
                decrypted,
            } => writeln!(
                out,
                r#"        <encryption encrypted="{encrypted}" handler="{}" version="{}" revision="{}" algorithm="{}" decrypted="{decrypted}"></encryption>"#,
                XmlEscapedAttr::new(handler.as_ref().map_or("", Identifier::as_str))?,
                version.map_or_else(String::new, |value| value.to_string()),
                revision.map_or_else(String::new, |value| value.to_string()),
                XmlEscapedAttr::new(algorithm.as_ref().map_or("", Identifier::as_str))?,
            )
            .map_err(write_error)?,
            ParseFact::Xmp { object, fact } => writeln!(
                out,
                r#"        <xmp object="{} {}" fact="{}"></xmp>"#,
                object.number,
                object.generation,
                XmlEscapedAttr::new(&xmp_fact_text(fact))?,
            )
            .map_err(write_error)?,
        }
    }
    writeln!(out, "      </parseFacts>").map_err(write_error)?;
    Ok(())
}

fn write_xml_warnings<W: Write>(
    warnings: &[ValidationWarning],
    out: &mut W,
    indent: usize,
) -> Result<()> {
    if warnings.is_empty() {
        return Ok(());
    }
    let spaces = " ".repeat(indent);
    writeln!(out, "{spaces}<warnings>").map_err(write_error)?;
    for warning in warnings {
        writeln!(
            out,
            "{spaces}  <warning>{}</warning>",
            XmlEscapedText::new(&warning_text(warning))?,
        )
        .map_err(write_error)?;
    }
    writeln!(out, "{spaces}</warnings>").map_err(write_error)?;
    Ok(())
}

fn write_xml_batch_summary<W: Write>(summary: &BatchSummary, out: &mut W) -> Result<()> {
    writeln!(
        out,
        r#"  <batchSummary totalJobs="{}" failedToParse="{}" encrypted="{}" incomplete="{}" internalErrors="{}">"#,
        summary.total_files,
        summary.parse_failures,
        summary.encrypted,
        summary.incomplete,
        summary.internal_errors,
    )
    .map_err(write_error)?;
    writeln!(
        out,
        r#"    <validationReports compliant="{}" nonCompliant="{}" failedJobs="{}">{}</validationReports>"#,
        summary.valid,
        summary.invalid,
        summary
            .parse_failures
            .saturating_add(summary.encrypted)
            .saturating_add(summary.incomplete)
            .saturating_add(summary.internal_errors),
        summary.valid.saturating_add(summary.invalid),
    )
    .map_err(write_error)?;
    writeln!(
        out,
        r#"    <duration elapsedMillis="{}"></duration>"#,
        summary.elapsed_millis,
    )
    .map_err(write_error)?;
    writeln!(out, "  </batchSummary>").map_err(write_error)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CheckCounts {
    passed: u64,
    failed: u64,
    unsupported: u64,
}

fn check_counts(report: &ValidationReport) -> CheckCounts {
    report
        .profile_reports
        .iter()
        .fold(CheckCounts::default(), |mut counts, profile| {
            let failed = profile.failed_rules;
            let unsupported = u64::try_from(profile.unsupported_rules.len()).unwrap_or(u64::MAX);
            counts.failed = counts.failed.saturating_add(failed);
            counts.unsupported = counts.unsupported.saturating_add(unsupported);
            counts.passed = counts
                .passed
                .saturating_add(profile.checks_executed.saturating_sub(failed));
            counts
        })
}

fn profile_list(report: &ValidationReport) -> String {
    let profiles = report
        .profile_reports
        .iter()
        .map(|profile| profile.profile.id.as_str())
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        String::from("-")
    } else {
        profiles.join(", ")
    }
}

fn source_name(source: &InputSummary) -> String {
    source.path.as_ref().map_or_else(
        || String::from("<memory>"),
        |path| path.display().to_string(),
    )
}

fn status_text(status: ValidationStatus) -> &'static str {
    match status {
        ValidationStatus::Valid => "valid",
        ValidationStatus::Invalid => "invalid",
        ValidationStatus::Encrypted => "encrypted",
        ValidationStatus::Incomplete => "incomplete",
        ValidationStatus::ParseFailed => "parse failed",
    }
}

fn repair_status_text(status: RepairStatus) -> &'static str {
    match status {
        RepairStatus::Succeeded => "succeeded",
        RepairStatus::NoAction => "no action",
        RepairStatus::Refused => "refused",
        RepairStatus::Failed => "failed",
    }
}

fn repair_action_kind(action: &RepairAction) -> &'static str {
    match action {
        RepairAction::CopiedUnchanged => "copiedUnchanged",
        RepairAction::MetadataRewritten { .. } => "metadataRewritten",
    }
}

fn repair_action_text(action: &RepairAction) -> String {
    match action {
        RepairAction::CopiedUnchanged => String::from("copied unchanged"),
        RepairAction::MetadataRewritten { description } => description.as_str().to_owned(),
    }
}

fn repair_refusal_kind(refusal: &RepairRefusal) -> &'static str {
    match refusal {
        RepairRefusal::ParseFailed { .. } => "parseFailed",
        RepairRefusal::Encrypted => "encrypted",
        RepairRefusal::AmbiguousFlavour { .. } => "ambiguousFlavour",
        RepairRefusal::UnsupportedValidationStatus { .. } => "unsupportedValidationStatus",
        RepairRefusal::OutputWouldModifyInput => "outputWouldModifyInput",
        RepairRefusal::InvalidOutputPath { .. } => "invalidOutputPath",
    }
}

fn repair_refusal_text(refusal: &RepairRefusal) -> String {
    match refusal {
        RepairRefusal::ParseFailed { reason } => {
            format!("input could not be parsed: {}", reason.as_str())
        }
        RepairRefusal::Encrypted => String::from("encrypted inputs are not repaired"),
        RepairRefusal::AmbiguousFlavour { selected } => {
            format!("repair requires exactly one selected flavour, got {selected}")
        }
        RepairRefusal::UnsupportedValidationStatus { status } => {
            format!(
                "metadata repair is unsupported for {} inputs",
                status_text(*status)
            )
        }
        RepairRefusal::OutputWouldModifyInput => {
            String::from("output path would modify input in place")
        }
        RepairRefusal::InvalidOutputPath { reason } => reason.as_str().to_owned(),
    }
}

fn exit_category_text(category: ExitCategory) -> &'static str {
    match category {
        ExitCategory::Success => "success",
        ExitCategory::ValidationFailed => "validation failed",
        ExitCategory::ProcessingFailed => "processing failed",
        ExitCategory::InternalError => "internal error",
    }
}

fn location_text(location: &ObjectLocation) -> String {
    if let Some(path) = &location.path {
        return path.to_string();
    }
    if let Some(object) = location.object {
        return format!("object {} {}", object.number, object.generation);
    }
    if let Some(offset) = location.offset {
        return format!("offset {offset}");
    }
    String::from("unknown")
}

fn assertion_message(assertion: &Assertion) -> &str {
    assertion
        .message
        .as_ref()
        .unwrap_or(&assertion.description)
        .as_str()
}

fn assertion_status_text(status: AssertionStatus) -> &'static str {
    match status {
        AssertionStatus::Passed => "passed",
        AssertionStatus::Failed => "failed",
    }
}

fn status_statement(status: ValidationStatus) -> &'static str {
    match status {
        ValidationStatus::Valid => "PDF file is compliant with Validation Profile requirements.",
        ValidationStatus::Invalid => {
            "PDF file is not compliant with Validation Profile requirements."
        }
        ValidationStatus::Encrypted => "PDF file is encrypted and could not be validated.",
        ValidationStatus::Incomplete => "Validation did not complete for all required rules.",
        ValidationStatus::ParseFailed => "PDF file could not be parsed.",
    }
}

fn xref_fact_text(fact: &XrefFact) -> String {
    match fact {
        XrefFact::EolMarkersComply => String::from("eolMarkersComply"),
        XrefFact::MalformedClassic => String::from("malformedClassic"),
        XrefFact::XrefStreamUnsupported => String::from("xrefStreamUnsupported"),
        XrefFact::XrefStreamParsed {
            entries,
            compressed_entries,
        } => format!("xrefStreamParsed entries={entries} compressedEntries={compressed_entries}"),
        XrefFact::PrevChain { offset } => format!("prevChain offset={offset}"),
        XrefFact::HybridReference { offset } => format!("hybridReference offset={offset}"),
        XrefFact::ObjectStreamParsed => String::from("objectStreamParsed"),
    }
}

fn stream_fact_text(fact: &StreamFact) -> String {
    match fact {
        StreamFact::Length {
            declared,
            discovered,
        } => format!("length declared={declared} discovered={discovered}"),
        StreamFact::KeywordSpacing {
            stream_keyword_crlf_compliant,
            endstream_keyword_eol_compliant,
        } => format!(
            "keywordSpacing streamKeywordCRLFCompliant={stream_keyword_crlf_compliant} \
             endstreamKeywordEolCompliant={endstream_keyword_eol_compliant}"
        ),
        StreamFact::Decoded { bytes } => format!("decoded bytes={bytes}"),
        StreamFact::FilterDecoded {
            filter,
            input_bytes,
            output_bytes,
        } => format!(
            "filterDecoded filter={} inputBytes={input_bytes} outputBytes={output_bytes}",
            filter.as_str()
        ),
        StreamFact::FilterMetadataMode { filter, bytes } => {
            format!(
                "filterMetadataMode filter={} bytes={bytes}",
                filter.as_str()
            )
        }
    }
}

fn xmp_fact_text(fact: &XmpFact) -> String {
    match fact {
        XmpFact::PacketParsed {
            bytes,
            namespaces,
            claims,
        } => format!("packetParsed bytes={bytes} namespaces={namespaces} claims={claims}"),
        XmpFact::MissingPacketWrapper => String::from("missingPacketWrapper"),
        XmpFact::FlavourClaim {
            family,
            display_flavour,
            namespace_uri,
        } => format!(
            "flavourClaim family={} displayFlavour={} namespaceUri={}",
            family.as_str(),
            display_flavour.as_str(),
            namespace_uri.as_str()
        ),
        XmpFact::Malformed { reason } => format!("malformed reason={}", reason.as_str()),
        XmpFact::HostileXmlRejected { reason } => {
            format!("hostileXmlRejected reason={}", reason.as_str())
        }
    }
}

fn warning_text(warning: &ValidationWarning) -> String {
    warning.message_text().to_string()
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn write_error(source: std::io::Error) -> PdfvError {
    ReportError::Write { source }.into()
}

fn format_optional_path(path: Option<&PathBuf>) -> String {
    path.map(|path| format!(" at {}", path.display()))
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug)]
struct XmlEscapedText<'a>(&'a str);

impl<'a> XmlEscapedText<'a> {
    fn new(value: &'a str) -> Result<Self> {
        ensure_xml_text(value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for XmlEscapedText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for character in self.0.chars() {
            match character {
                '&' => formatter.write_str("&amp;")?,
                '<' => formatter.write_str("&lt;")?,
                '>' => formatter.write_str("&gt;")?,
                '"' => formatter.write_str("&quot;")?,
                '\'' => formatter.write_str("&apos;")?,
                _ => formatter.write_str(character.encode_utf8(&mut [0; 4]))?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct XmlEscapedAttr<'a>(&'a str);

impl<'a> XmlEscapedAttr<'a> {
    fn new(value: &'a str) -> Result<Self> {
        ensure_xml_text(value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for XmlEscapedAttr<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        XmlEscapedText(self.0).fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug)]
struct HtmlEscapedText<'a>(&'a str);

impl<'a> HtmlEscapedText<'a> {
    fn new(value: &'a str) -> Result<Self> {
        ensure_xml_text(value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for HtmlEscapedText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        XmlEscapedText(self.0).fmt(formatter)
    }
}

fn ensure_xml_text(value: &str) -> Result<()> {
    if value.chars().all(is_xml_char) {
        return Ok(());
    }
    Err(ReportError::Xml {
        message: BoundedText::unchecked("text contains characters forbidden by XML 1.0"),
    }
    .into())
}

fn is_xml_char(character: char) -> bool {
    matches!(character, '\u{09}' | '\u{0A}' | '\u{0D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        error::Error as StdError,
        num::{NonZeroU32, NonZeroU64},
        path::PathBuf,
        time::Duration,
    };

    use super::{
        Assertion, AssertionStatus, BatchReport, BoundedText, ErrorArgument, ExitCategory,
        FeatureObject, FeatureReport, FeatureValue, HtmlReportWriter, Identifier, InputKind,
        InputSummary, JsonReportWriter, MaxDisplayedFailures, MetadataRepairOptions,
        MetadataRepairer, ObjectLocation, ObjectTypeName, PdfVersion, PolicyReport,
        PolicyRuleResult, ProfileIdentity, ProfileReport, PropertyName, RawXmlReportWriter,
        RepairAction, RepairBatchReport, RepairRefusal, RepairReport, RepairStatus, ReportFormat,
        ReportWriter, RuleId, TextReportWriter, ValidationOptions, ValidationReport,
        ValidationStatus, XmlReportWriter,
    };

    fn sample_report() -> std::result::Result<ValidationReport, Box<dyn StdError>> {
        let profile_id = Identifier::new("pdfa-1b")?;
        let rule_id = RuleId(Identifier::new("6.1.2-1")?);
        Ok(ValidationReport::builder()
            .engine_version("0.1.0".to_owned())
            .source(InputSummary::new(InputKind::Memory, None, Some(42)))
            .status(ValidationStatus::Invalid)
            .flavours(vec![super::ValidationFlavour::new(
                "pdfa",
                NonZeroU32::MIN,
                "b",
            )?])
            .profile_reports(vec![
                ProfileReport::builder()
                    .profile(ProfileIdentity {
                        id: profile_id.clone(),
                        name: BoundedText::new("PDF/A-1B", 64)?,
                        version: None,
                    })
                    .is_compliant(false)
                    .checks_executed(1)
                    .rules_executed(1)
                    .failed_rules(1)
                    .failed_assertions(vec![Assertion {
                        ordinal: NonZeroU64::MIN,
                        rule_id,
                        status: AssertionStatus::Failed,
                        description: BoundedText::new("Header must start at byte zero", 128)?,
                        location: ObjectLocation {
                            object: None,
                            offset: Some(0),
                            path: None,
                        },
                        object_context: None,
                        message: Some(BoundedText::new("Header offset is non-zero", 128)?),
                        error_arguments: vec![ErrorArgument {
                            name: Identifier::new("offset")?,
                            value: BoundedText::new("12", 16)?,
                        }],
                    }])
                    .passed_assertions(Vec::new())
                    .unsupported_rules(Vec::new())
                    .build(),
            ])
            .parse_facts(vec![super::ParseFact::Header {
                offset: 12,
                version: PdfVersion { major: 1, minor: 7 },
                had_leading_bytes: true,
            }])
            .warnings(Vec::new())
            .task_durations(Vec::new())
            .build())
    }

    fn sample_feature_policy_report() -> std::result::Result<ValidationReport, Box<dyn StdError>> {
        let mut report = sample_report()?;
        let mut properties = BTreeMap::new();
        properties.insert(PropertyName::new("hasMetadata")?, FeatureValue::Bool(false));
        report.feature_report = Some(
            FeatureReport::builder()
                .objects(vec![
                    FeatureObject::builder()
                        .family(ObjectTypeName::new("catalog".to_owned())?)
                        .location(ObjectLocation {
                            object: None,
                            offset: None,
                            path: Some(BoundedText::new("root/catalog[0]", 64)?),
                        })
                        .context(BoundedText::new("root/catalog[0]", 64)?)
                        .properties(properties)
                        .build(),
                ])
                .visited_objects(1)
                .selected_families(vec![ObjectTypeName::new("catalog".to_owned())?])
                .truncated(false)
                .build(),
        );
        report.policy_report = Some(
            PolicyReport::builder()
                .name(Some(BoundedText::new("catalog-policy", 64)?))
                .is_compliant(true)
                .results(vec![
                    PolicyRuleResult::builder()
                        .id(Identifier::new("catalog-has-no-metadata")?)
                        .description(BoundedText::new("Catalog metadata is absent", 128)?)
                        .passed(true)
                        .matches(1)
                        .message(BoundedText::new(
                            "policy rule catalog-has-no-metadata passed with 1 matching feature \
                             objects",
                            128,
                        )?)
                        .build(),
                ])
                .build(),
        );
        Ok(report)
    }

    fn sample_repair_report() -> RepairReport {
        RepairReport::builder()
            .engine_version("0.1.0".to_owned())
            .source(InputSummary::new(
                InputKind::File,
                Some(PathBuf::from("input.pdf")),
                Some(42),
            ))
            .output_path(Some(PathBuf::from("out/repaired-input.pdf")))
            .status(RepairStatus::NoAction)
            .actions(vec![RepairAction::CopiedUnchanged])
            .refusal(None)
            .warnings(Vec::new())
            .task_durations(Vec::new())
            .build()
    }

    #[test]
    fn test_should_apply_validation_options_defaults() {
        let options = ValidationOptions::default();

        assert!(options.report_parse_warnings);
        assert!(!options.record_passed_assertions);
        assert_eq!(options.max_failed_assertions_per_rule.get(), 1);
    }

    #[test]
    fn test_should_reject_zero_max_displayed_failures() {
        let result = MaxDisplayedFailures::try_from(0);

        assert!(result.is_err());
    }

    #[test]
    fn test_should_reject_invalid_identifier() {
        let result = Identifier::new("bad identifier");

        assert!(result.is_err());
    }

    #[test]
    fn test_should_serialize_validation_report_as_camel_case_json()
    -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let json = serde_json::to_string_pretty(&report)?;
        let expected = r#"{
  "engineVersion": "0.1.0",
  "source": {
    "kind": "memory",
    "bytes": 42
  },
  "status": "invalid",
  "flavours": [
    {
      "family": "pdfa",
      "part": 1,
      "conformance": "b"
    }
  ],
  "profileReports": [
    {
      "profile": {
        "id": "pdfa-1b",
        "name": "PDF/A-1B"
      },
      "isCompliant": false,
      "checksExecuted": 1,
      "rulesExecuted": 1,
      "failedRules": 1,
      "failedAssertions": [
        {
          "ordinal": 1,
          "ruleId": "6.1.2-1",
          "status": "failed",
          "description": "Header must start at byte zero",
          "location": {
            "offset": 0
          },
          "message": "Header offset is non-zero",
          "errorArguments": [
            {
              "name": "offset",
              "value": "12"
            }
          ]
        }
      ],
      "passedAssertions": [],
      "unsupportedRules": []
    }
  ],
  "parseFacts": [
    {
      "kind": "header",
      "offset": 12,
      "version": {
        "major": 1,
        "minor": 7
      },
      "hadLeadingBytes": true
    }
  ],
  "warnings": [],
  "taskDurations": []
}"#;

        assert_eq!(json, expected);
        Ok(())
    }

    #[test]
    fn test_should_write_compact_json_report() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let mut output = Vec::new();

        JsonReportWriter::compact()
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let json = String::from_utf8(output)?;
        assert!(json.contains("\"engineVersion\":\"0.1.0\""));
        Ok(())
    }

    #[test]
    fn test_should_write_text_report() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let mut output = Vec::new();

        TextReportWriter
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let text = String::from_utf8(output)?;
        let expected = "\
<memory>: invalid
profiles: pdfa-1b
checks: 0 passed, 1 failed, 0 unsupported
first failures:
  6.1.2-1 at offset 0: Header offset is non-zero
";
        assert_eq!(text, expected);
        Ok(())
    }

    #[test]
    fn test_should_write_xml_report() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let mut output = Vec::new();

        XmlReportWriter
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let xml = String::from_utf8(output)?;
        assert!(xml.contains(r#"<?xml version="1.0" encoding="utf-8"?>"#));
        assert!(xml.contains("<report>"));
        assert!(xml.contains(r#"<validationReport profileName="PDF/A-1B""#));
        assert!(xml.contains(r#"<details passedRules="0" failedRules="1""#));
        assert!(xml.contains(r#"<check ruleId="6.1.2-1" status="failed" location="offset 0">"#));
        assert!(xml.contains(r#"<batchSummary totalJobs="1""#));
        Ok(())
    }

    #[test]
    fn test_should_write_raw_xml_report_with_feature_and_policy_sections()
    -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_feature_policy_report()?;
        let mut output = Vec::new();

        RawXmlReportWriter
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let xml = String::from_utf8(output)?;
        let expected = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<rawReport engine="pdfv-core" version="{version}">
  <processorConfig tasks="validation,features,policy"></processorConfig>
  <processorResults>
    <processorResult status="invalid">
      <item size="42">
        <name>&lt;memory&gt;</name>
      </item>
      <validationReport profileName="PDF/A-1B" statement="PDF file is not compliant with Validation Profile requirements." isCompliant="false">
        <details passedRules="0" failedRules="1" passedChecks="0" failedChecks="1" unsupportedRules="0"></details>
        <failedChecks>
          <check ruleId="6.1.2-1" status="failed" location="offset 0">
            <description>Header must start at byte zero</description>
            <message>Header offset is non-zero</message>
            <errorArguments>
              <argument name="offset">12</argument>
            </errorArguments>
          </check>
        </failedChecks>
      </validationReport>
      <featureReport visitedObjects="1" extractedObjects="1" truncated="false">
        <featureObject family="catalog" location="root/catalog[0]">
          <property name="hasMetadata">
            <value type="bool">false</value>
          </property>
        </featureObject>
      </featureReport>
      <policyReport name="catalog-policy" isCompliant="true">
        <rule id="catalog-has-no-metadata" passed="true" matches="1">
          <description>Catalog metadata is absent</description>
          <message>policy rule catalog-has-no-metadata passed with 1 matching feature objects</message>
        </rule>
      </policyReport>
      <parseFacts>
        <header offset="12" version="1.7" hadLeadingBytes="true"></header>
      </parseFacts>
    </processorResult>
  </processorResults>
  <batchSummary totalJobs="1" failedToParse="0" encrypted="0" incomplete="0" internalErrors="0">
    <validationReports compliant="0" nonCompliant="1" failedJobs="0">1</validationReports>
    <duration elapsedMillis="0"></duration>
  </batchSummary>
</rawReport>
"#,
            version = super::ENGINE_VERSION,
        );
        assert_eq!(xml, expected);
        Ok(())
    }

    #[test]
    fn test_should_write_static_html_report() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let mut output = Vec::new();

        HtmlReportWriter
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let html = String::from_utf8(output)?;
        let expected = "\
<!doctype html>
<html lang=\"en\"><head><meta charset=\"utf-8\"><title>pdfv validation \
                        report</title><style>body{font-family:system-ui,sans-serif;margin:2rem;\
                        color:#1f2937}table{border-collapse:collapse;width:100%}th,td{border:1px \
                        solid #d1d5db;padding:.4rem;text-align:left}th{background:#f3f4f6}</\
                        style></head><body>
<h1>Validation Report</h1>
<p>0 valid, 1 invalid, 0 parse failed, 0 encrypted, 0 incomplete.</p>
<table><thead><tr><th>Input</th><th>Status</th><th>Profiles</th><th>Features</th><th>Policy</th></\
                        tr></thead><tbody>
<tr><td>&lt;memory&gt;</td><td>invalid</td><td>pdfa-1b</td><td>-</td><td>-</td></tr>
</tbody></table>
</body></html>
";
        assert_eq!(html, expected);
        Ok(())
    }

    #[test]
    fn test_should_serialize_repair_report_and_summary()
    -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_repair_report();
        let json = serde_json::to_string_pretty(&report)?;
        assert!(json.contains(r#""status": "noAction""#));
        assert!(json.contains(r#""kind": "copiedUnchanged""#));

        let refused = RepairReport::builder()
            .engine_version("0.1.0".to_owned())
            .source(InputSummary::new(
                InputKind::File,
                Some(PathBuf::from("bad.pdf")),
                None,
            ))
            .output_path(None)
            .status(RepairStatus::Refused)
            .actions(Vec::new())
            .refusal(Some(RepairRefusal::Encrypted))
            .warnings(Vec::new())
            .task_durations(Vec::new())
            .build();
        let batch =
            RepairBatchReport::from_items(vec![report, refused], Vec::new(), Duration::ZERO);

        assert_eq!(batch.summary.no_action, 1);
        assert_eq!(batch.summary.refused, 1);
        assert_eq!(
            batch.summary.worst_exit_category,
            ExitCategory::ProcessingFailed
        );
        Ok(())
    }

    #[test]
    fn test_should_write_repair_raw_xml_and_html() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_repair_report();
        let mut raw = Vec::new();
        let mut html = Vec::new();

        ReportFormat::RawXml
            .write_repair_report(&report, &mut raw)
            .map_err(Box::<dyn StdError>::from)?;
        ReportFormat::Html
            .write_repair_report(&report, &mut html)
            .map_err(Box::<dyn StdError>::from)?;

        let raw = String::from_utf8(raw)?;
        let html = String::from_utf8(html)?;
        let expected_raw = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<rawRepairReport engine="pdfv-core" version="{version}">
  <processorConfig tasks="metadata"></processorConfig>
  <items>
    <repairItem status="no action">
      <item size="42">
        <name>input.pdf</name>
      </item>
      <output>out/repaired-input.pdf</output>
      <actions>
        <action kind="copiedUnchanged">copied unchanged</action>
      </actions>
    </repairItem>
  </items>
  <repairSummary totalJobs="1" succeeded="0" noAction="1" refused="0" failed="0" elapsedMillis="0"></repairSummary>
</rawRepairReport>
"#,
            version = super::ENGINE_VERSION,
        );
        let expected_html =
            "\
<!doctype html>
<html lang=\"en\"><head><meta charset=\"utf-8\"><title>pdfv metadata repair \
             report</title><style>body{font-family:system-ui,sans-serif;margin:2rem;color:#\
             1f2937}table{border-collapse:collapse;width:100%}th,td{border:1px solid \
             #d1d5db;padding:.4rem;text-align:left}th{background:#f3f4f6}</style></head><body>
<h1>Metadata Repair Report</h1>
<p>0 repaired, 1 unchanged, 0 refused, 0 failed.</p>
<table><thead><tr><th>Input</th><th>Status</th><th>Output</th><th>Reason</th></tr></thead><tbody>
<tr><td>input.pdf</td><td>no action</td><td>out/repaired-input.pdf</td><td></td></tr>
</tbody></table>
</body></html>
";
        assert_eq!(raw, expected_raw);
        assert_eq!(html, expected_html);
        Ok(())
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "unit test creates local repair files synchronously"
    )]
    fn test_should_refuse_repair_when_output_already_exists_without_removing_it()
    -> std::result::Result<(), Box<dyn StdError>> {
        let temp = tempfile::tempdir()?;
        let input = temp.path().join("input.pdf");
        let output_dir = temp.path().join("out");
        let output = output_dir.join("input.pdf");
        std::fs::create_dir(&output_dir)?;
        std::fs::write(&input, b"not a valid pdf")?;
        std::fs::write(&output, b"existing output")?;
        let repairer = MetadataRepairer::new(MetadataRepairOptions::new(
            ValidationOptions::default(),
            &output_dir,
            "",
        )?)?;

        let report = repairer.repair_path(&input)?;

        assert_eq!(report.status, RepairStatus::Refused);
        assert!(matches!(
            report.refusal,
            Some(RepairRefusal::InvalidOutputPath { .. })
        ));
        assert_eq!(std::fs::read(&output)?, b"existing output");
        Ok(())
    }

    #[test]
    fn test_should_reject_xml_forbidden_text() -> std::result::Result<(), Box<dyn StdError>> {
        let mut report = sample_report()?;
        let Some(profile) = report.profile_reports.first_mut() else {
            return Err("sample report must contain profile".into());
        };
        profile.profile.name = BoundedText::unchecked("bad\u{1}profile");
        let mut output = Vec::new();

        let result = XmlReportWriter.write_report(&report, &mut output);

        assert!(matches!(
            result,
            Err(super::PdfvError::Report(super::ReportError::Xml { .. }))
        ));
        Ok(())
    }

    #[test]
    fn test_should_dispatch_pretty_json_report_format() -> std::result::Result<(), Box<dyn StdError>>
    {
        let report = sample_report()?;
        let mut output = Vec::new();

        ReportFormat::JsonPretty
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let json = String::from_utf8(output)?;
        assert!(json.contains("\n  \"engineVersion\": \"0.1.0\""));
        Ok(())
    }

    #[test]
    fn test_should_dispatch_xml_report_format() -> std::result::Result<(), Box<dyn StdError>> {
        let report = sample_report()?;
        let mut output = Vec::new();

        ReportFormat::Xml
            .write_report(&report, &mut output)
            .map_err(Box::<dyn StdError>::from)?;

        let xml = String::from_utf8(output)?;
        assert!(xml.contains("<validationReport"));
        Ok(())
    }

    #[test]
    fn test_should_compute_batch_summary() -> std::result::Result<(), Box<dyn StdError>> {
        let valid = ValidationReport::builder()
            .engine_version("0.1.0".to_owned())
            .source(InputSummary::new(InputKind::Memory, None, Some(42)))
            .status(ValidationStatus::Valid)
            .flavours(Vec::new())
            .profile_reports(Vec::new())
            .parse_facts(Vec::new())
            .warnings(Vec::new())
            .task_durations(Vec::new())
            .build();
        let invalid = sample_report()?;

        let batch = BatchReport::from_items(vec![valid, invalid], Vec::new(), Duration::ZERO);

        assert_eq!(batch.summary.total_files, 2);
        assert_eq!(batch.summary.valid, 1);
        assert_eq!(batch.summary.invalid, 1);
        assert_eq!(
            batch.summary.worst_exit_category,
            ExitCategory::ValidationFailed
        );
        Ok(())
    }
}
