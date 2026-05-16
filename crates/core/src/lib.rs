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

mod parser;
mod profile;
mod validation;

use std::{
    fmt,
    io::Write,
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    time::Duration,
};

pub use parser::{
    CosObject, Dictionary, IndirectObject, ObjectStore, ParsedDocument, Parser, PdfName, PdfSource,
    PdfString, StreamObject, Trailer,
};
pub use profile::{
    BinaryOp, BuiltinFunction, BuiltinProfileRepository, ErrorTemplate, ModelValue, ObjectTypeName,
    ProfileRepository, PropertyName, PropertyPath, Rule, RuleEvaluator, RuleExpr, RuleOutcome,
    UnaryOp, ValidationProfile,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_builder::TypedBuilder;
pub use validation::{
    AnnotationModel, CatalogModel, ContentStreamModel, FontModel, InputName, LinkName,
    MetadataModel, ModelGraph, ModelLinks, ModelObject, ModelObjectRef, ObjectIdentity,
    OutputIntentModel, PageModel, Validator,
};

/// Current library version embedded in generated reports.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 4096;

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
    /// Maximum assertion details retained per failed rule.
    #[builder(default)]
    pub max_failed_assertions_per_rule: MaxDisplayedFailures,
    /// Whether passed assertion details are recorded.
    #[builder(default)]
    pub record_passed_assertions: bool,
    /// Whether recoverable parser warnings are included in the report.
    #[builder(default = true)]
    pub report_parse_warnings: bool,
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
        Self::Auto { default: None }
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
    /// Maximum declared stream bytes.
    pub max_stream_declared_bytes: u64,
    /// Maximum decoded stream bytes.
    pub max_stream_decode_bytes: u64,
    /// Maximum retained parse facts.
    pub max_parse_facts: usize,
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
            max_string_bytes: 1_048_576,
            max_stream_declared_bytes: 128 * 1024 * 1024,
            max_stream_decode_bytes: 256 * 1024 * 1024,
            max_parse_facts: 100_000,
        }
    }
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
    /// Task duration measurements.
    pub task_durations: Vec<TaskDuration>,
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
        for report in items {
            match report.status {
                ValidationStatus::Valid => summary.valid = summary.valid.saturating_add(1),
                ValidationStatus::Invalid => summary.invalid = summary.invalid.saturating_add(1),
                ValidationStatus::ParseFailed => {
                    summary.parse_failures = summary.parse_failures.saturating_add(1);
                }
                ValidationStatus::Encrypted => {
                    summary.encrypted = summary.encrypted.saturating_add(1);
                }
                ValidationStatus::Incomplete => {
                    summary.incomplete = summary.incomplete.saturating_add(1);
                }
            }
        }
        summary.worst_exit_category = if summary.parse_failures > 0
            || summary.encrypted > 0
            || summary.incomplete > 0
            || summary.internal_errors > 0
        {
            ExitCategory::ProcessingFailed
        } else if summary.invalid > 0 {
            ExitCategory::ValidationFailed
        } else {
            ExitCategory::Success
        };
        summary
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
    if !report.warnings.is_empty() {
        writeln!(out, "warnings: {}", report.warnings.len()).map_err(write_error)?;
    }
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
