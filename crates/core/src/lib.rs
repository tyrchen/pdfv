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

mod generated_profiles;
mod parser;
mod profile;
mod validation;
mod xmp;

use std::{
    collections::BTreeMap,
    fmt,
    io::Write,
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
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
    BinaryOp, BuiltinFunction, BuiltinProfileRepository, ErrorTemplate, ModelValue, ObjectTypeName,
    ProfileCatalogEntry, ProfileImportSummary, ProfileRepository, PropertyName, PropertyPath, Rule,
    RuleEvaluator, RuleExpr, RuleOutcome, UnaryOp, ValidationProfile, display_flavour,
    import_verapdf_profile_xml,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_builder::TypedBuilder;
pub use validation::{
    AnnotationModel, CatalogModel, ContentStreamModel, FeatureSelection, FontModel, InputName,
    LinkName, MetadataModel, ModelGraph, ModelObject, ModelObjectRef, ObjectIdentity,
    OutputIntentModel, PageModel, Validator,
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
    match warning {
        ValidationWarning::ParseFactCapReached { cap } => {
            format!("parse fact cap reached: {cap}")
        }
        ValidationWarning::IncompatibleProfile { profile_id, reason } => {
            format!(
                "incompatible profile {}: {}",
                profile_id.as_str(),
                reason.as_str()
            )
        }
        ValidationWarning::AutoDetection { message } => {
            format!("auto detection: {}", message.as_str())
        }
        ValidationWarning::General { message } => message.as_str().to_owned(),
    }
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
        error::Error as StdError,
        num::{NonZeroU32, NonZeroU64},
        time::Duration,
    };

    use super::{
        Assertion, AssertionStatus, BatchReport, BoundedText, ErrorArgument, ExitCategory,
        Identifier, InputKind, InputSummary, JsonReportWriter, MaxDisplayedFailures,
        ObjectLocation, PdfVersion, ProfileIdentity, ProfileReport, ReportFormat, ReportWriter,
        RuleId, TextReportWriter, ValidationOptions, ValidationReport, ValidationStatus,
        XmlReportWriter,
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
