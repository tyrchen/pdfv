//! Release-oracle corpus manifests, summaries, and drift classification.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

use crate::{
    BoundedText, ConfigError, CorpusOutcome, FlavourSelection, Identifier, Result,
    ValidationFlavour, ValidationOptions,
};

const MAX_MANIFEST_ROWS: usize = 100_000;
const MAX_EXPECTED_FEATURES: usize = 64;
const MAX_PRODUCER_BYTES: usize = 256;
const MAX_REASON_BYTES: usize = 1024;

/// Oracle corpus manifest row identifier.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CorpusRowId(Identifier);

impl CorpusRowId {
    /// Creates a corpus row id.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the id violates the identifier policy.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ConfigError> {
        Ok(Self(Identifier::new(value)?))
    }

    /// Returns the row id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for CorpusRowId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for CorpusRowId {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CorpusRowId> for String {
    fn from(value: CorpusRowId) -> Self {
        value.0.into()
    }
}

/// Relative corpus path from a configured corpus root.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CorpusPath(BoundedText);

impl CorpusPath {
    /// Creates a relative corpus path.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the path is empty, absolute, contains NUL, or
    /// attempts parent-directory traversal.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ConfigError> {
        let value = value.into();
        let invalid = value.is_empty()
            || value.len() > 1024
            || value.contains('\0')
            || value.starts_with('/')
            || value.split('/').any(|part| part == ".." || part.is_empty());
        if invalid {
            return Err(ConfigError::InvalidValue {
                field: "path",
                reason: BoundedText::unchecked("path must be a bounded relative corpus path"),
            });
        }
        Ok(Self(BoundedText::new(value, 1024)?))
    }

    /// Returns the relative path as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for CorpusPath {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CorpusPath> for String {
    fn from(value: CorpusPath) -> Self {
        value.0.into()
    }
}

/// Semantic family covered by a corpus row.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct SemanticFamilyName(Identifier);

impl SemanticFamilyName {
    /// Creates a semantic family name.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the name violates the identifier policy.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, ConfigError> {
        Ok(Self(Identifier::new(value)?))
    }

    /// Returns the semantic family name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for SemanticFamilyName {
    type Error = ConfigError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SemanticFamilyName> for String {
    fn from(value: SemanticFamilyName) -> Self {
        value.0.into()
    }
}

/// Oracle corpus tier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum CorpusTier {
    /// T0 generated fixtures.
    T0Generated,
    /// T1 checked-in fixtures.
    T1CheckedIn,
    /// T2 redistributable public conformance fixtures.
    T2PublicConformance,
    /// T3 private or user-provided real-world corpus.
    T3RealWorld,
    /// T4 adversarial safety corpus.
    T4Adversarial,
}

/// Corpus file size bucket.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum SizeBucket {
    /// Tiny fixture under 10 KiB.
    Tiny,
    /// Small fixture under 1 MiB.
    Small,
    /// Medium fixture under 25 MiB.
    Medium,
    /// Large fixture under 250 MiB.
    Large,
    /// Huge fixture at or above 250 MiB.
    Huge,
}

/// Corpus encryption class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum EncryptionClass {
    /// No encryption.
    Unencrypted,
    /// Encrypted with an empty user password.
    EmptyPassword,
    /// Encrypted with a non-empty password.
    PasswordProtected,
    /// Encryption is expected to be unsupported by pdfv.
    Unsupported,
    /// Encryption state is unknown.
    Unknown,
}

/// Corpus tagging class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum TaggingClass {
    /// No tagging is expected.
    Untagged,
    /// Document is tagged.
    Tagged,
    /// Document is partially tagged.
    PartiallyTagged,
    /// Tagging state is unknown.
    Unknown,
}

/// Corpus license class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum CorpusLicenseClass {
    /// Project-owned or generated fixture.
    ProjectFixture,
    /// Redistributable public fixture.
    Redistributable,
    /// Private T3 fixture that must not be committed.
    Private,
    /// License status is unknown.
    Unknown,
}

/// Profile selection policy for a live oracle row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum OracleProfilePolicy {
    /// Auto-detect with pdfv's default PDF/A-1b fallback.
    AutoDefaultPdfa1b,
    /// Auto-detect with an explicit fallback flavour.
    Auto {
        /// Optional default flavour used when detection is inconclusive.
        default: Option<ValidationFlavour>,
    },
    /// Validate against an explicit built-in flavour.
    Explicit {
        /// Selected flavour.
        flavour: ValidationFlavour,
    },
}

impl OracleProfilePolicy {
    /// Builds validation options for the policy.
    #[must_use]
    pub fn validation_options(&self) -> ValidationOptions {
        match self {
            Self::AutoDefaultPdfa1b => ValidationOptions::default(),
            Self::Auto { default } => ValidationOptions::builder()
                .flavour(FlavourSelection::Auto {
                    default: default.clone(),
                })
                .build(),
            Self::Explicit { flavour } => ValidationOptions::builder()
                .flavour(FlavourSelection::Explicit {
                    flavour: flavour.clone(),
                })
                .build(),
        }
    }

    /// Returns the profile family key used for summaries.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the generated summary key violates the
    /// identifier policy.
    pub fn profile_family_key(&self) -> std::result::Result<Identifier, ConfigError> {
        match self {
            Self::AutoDefaultPdfa1b | Self::Auto { .. } => Identifier::new("auto"),
            Self::Explicit { flavour } => Identifier::new(flavour.family.as_str()),
        }
    }

    /// Returns the veraPDF CLI flavour code when the policy has a direct equivalent.
    #[must_use]
    pub fn verapdf_flavour_code(&self) -> Option<String> {
        match self {
            Self::AutoDefaultPdfa1b | Self::Auto { .. } => None,
            Self::Explicit { flavour } => verapdf_flavour_code(flavour),
        }
    }
}

/// One manifest row for live oracle execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleCorpusRow {
    /// Stable row id.
    pub id: CorpusRowId,
    /// Relative path from the configured corpus root.
    pub path: CorpusPath,
    /// Corpus tier.
    pub tier: CorpusTier,
    /// Profile selection policy.
    pub profile_policy: OracleProfilePolicy,
    /// Expected semantic families exercised by the fixture.
    pub expected_features: Vec<SemanticFamilyName>,
    /// Optional bounded PDF producer string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<BoundedText>,
    /// Size bucket.
    pub size_bucket: SizeBucket,
    /// Encryption class.
    pub encryption: EncryptionClass,
    /// Tagging class.
    pub tagging: TaggingClass,
    /// License class.
    pub license: CorpusLicenseClass,
    /// Optional pre-triaged classification for a known mismatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_classification: Option<OracleDriftClassification>,
    /// Required note when `expectedClassification` is a non-match class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_note: Option<BoundedText>,
}

/// Live oracle manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleCorpusManifest {
    /// Manifest schema version.
    pub schema_version: Identifier,
    /// Relative corpus root from the manifest file directory.
    pub corpus_root: CorpusPath,
    /// Manifest rows.
    pub rows: Vec<OracleCorpusRow>,
}

impl OracleCorpusManifest {
    /// Validates manifest-level collection bounds and tier/license invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if any manifest invariant is violated.
    pub fn validate(&self) -> Result<()> {
        if self.rows.len() > MAX_MANIFEST_ROWS {
            return Err(ConfigError::InvalidValue {
                field: "rows",
                reason: BoundedText::unchecked("manifest contains too many rows"),
            }
            .into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for row in &self.rows {
            validate_row(row, &mut seen)?;
        }
        Ok(())
    }
}

/// Manual or computed drift classification for a live oracle row.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum OracleDriftClassification {
    /// pdfv and veraPDF agree on outcome class.
    Match,
    /// pdfv is incomplete because a known unsupported rule blocks a decision.
    ExpectedDriftUnsupportedRule,
    /// Difference is tied to an explicit non-goal or out-of-scope surface.
    ExpectedDriftScope,
    /// Difference is caused by a clearly reported pdfv resource limit.
    ExpectedDriftLimit,
    /// Upstream behavior is version-sensitive or ambiguous and reviewed.
    VeraPdfAmbiguity,
    /// Any untriaged mismatch.
    UnexpectedDrift,
    /// Confirmed incorrect pdfv behavior.
    PdfvBug,
}

/// Process execution status for one side of an oracle row.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum OracleExecutionStatus {
    /// Process completed and produced an outcome.
    Completed,
    /// Process timed out.
    Timeout,
    /// Process failed before an outcome could be derived.
    Failed,
}

/// Observed pdfv and veraPDF outcomes for one manifest row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OracleRowObservation {
    /// veraPDF outcome when available.
    pub vera_pdf_outcome: Option<CorpusOutcome>,
    /// pdfv outcome when available.
    pub pdfv_outcome: Option<CorpusOutcome>,
    /// pdfv unsupported-rule count.
    pub pdfv_unsupported_rules: u64,
    /// veraPDF execution status.
    pub vera_pdf_status: OracleExecutionStatus,
    /// pdfv execution status.
    pub pdfv_status: OracleExecutionStatus,
}

/// Report metadata captured for a live oracle run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleReportMetadata {
    /// pdfv engine version.
    pub pdfv_version: BoundedText,
    /// pdfv git commit or `unknown`.
    pub pdfv_commit: BoundedText,
    /// veraPDF version banner.
    pub vera_pdf_version: BoundedText,
    /// Java version banner.
    pub java_version: BoundedText,
    /// Operating system family.
    pub os: BoundedText,
    /// CPU architecture.
    pub arch: BoundedText,
    /// Vendored source pins.
    pub vendor_pins: BTreeMap<String, String>,
}

impl OracleReportMetadata {
    /// Creates oracle report metadata.
    #[must_use]
    pub fn new(
        pdfv_version: BoundedText,
        pdfv_commit: BoundedText,
        vera_pdf_version: BoundedText,
        java_version: BoundedText,
        os: BoundedText,
        arch: BoundedText,
        vendor_pins: BTreeMap<String, String>,
    ) -> Self {
        Self {
            pdfv_version,
            pdfv_commit,
            vera_pdf_version,
            java_version,
            os,
            arch,
            vendor_pins,
        }
    }
}

/// One live oracle result row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleCorpusResultRow {
    /// Stable row id.
    pub id: CorpusRowId,
    /// Manifest-relative corpus path.
    pub path: CorpusPath,
    /// Corpus tier.
    pub tier: CorpusTier,
    /// Profile selection policy.
    pub profile_policy: OracleProfilePolicy,
    /// Profile family summary key.
    pub profile_family: Identifier,
    /// Expected semantic families.
    pub expected_features: Vec<SemanticFamilyName>,
    /// Optional bounded producer string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<BoundedText>,
    /// Size bucket.
    pub size_bucket: SizeBucket,
    /// Encryption class.
    pub encryption: EncryptionClass,
    /// Tagging class.
    pub tagging: TaggingClass,
    /// License class.
    pub license: CorpusLicenseClass,
    /// veraPDF outcome when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vera_pdf_outcome: Option<CorpusOutcome>,
    /// pdfv outcome when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdfv_outcome: Option<CorpusOutcome>,
    /// veraPDF execution status.
    pub vera_pdf_status: OracleExecutionStatus,
    /// pdfv execution status.
    pub pdfv_status: OracleExecutionStatus,
    /// Final drift classification.
    pub classification: OracleDriftClassification,
    /// Whether this row is false-compliant.
    pub false_compliant: bool,
    /// Whether this row blocks release readiness.
    pub release_blocking: bool,
    /// pdfv unsupported-rule count for diagnostics.
    pub pdfv_unsupported_rules: u64,
    /// Optional bounded classification or process-failure reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BoundedText>,
}

impl OracleCorpusResultRow {
    /// Builds a result row from a manifest row and observed outcomes.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if generated bounded fields violate their caps.
    pub fn from_observation(
        row: &OracleCorpusRow,
        observation: OracleRowObservation,
    ) -> Result<Self> {
        let classification = classify_oracle_observation(row, &observation);
        let false_compliant = is_false_compliant(&observation);
        let release_blocking = false_compliant
            || matches!(
                classification,
                OracleDriftClassification::UnexpectedDrift | OracleDriftClassification::PdfvBug
            )
            || matches!(observation.vera_pdf_status, OracleExecutionStatus::Timeout)
            || matches!(observation.pdfv_status, OracleExecutionStatus::Timeout);
        Ok(Self {
            id: row.id.clone(),
            path: row.path.clone(),
            tier: row.tier,
            profile_policy: row.profile_policy.clone(),
            profile_family: row.profile_policy.profile_family_key()?,
            expected_features: row.expected_features.clone(),
            producer: row.producer.clone(),
            size_bucket: row.size_bucket,
            encryption: row.encryption,
            tagging: row.tagging,
            license: row.license,
            vera_pdf_outcome: observation.vera_pdf_outcome,
            pdfv_outcome: observation.pdfv_outcome,
            vera_pdf_status: observation.vera_pdf_status,
            pdfv_status: observation.pdfv_status,
            classification,
            false_compliant,
            release_blocking,
            pdfv_unsupported_rules: observation.pdfv_unsupported_rules,
            reason: result_reason(row, &observation, classification)?,
        })
    }
}

/// Live oracle corpus report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleCorpusReport {
    /// Report schema version.
    pub schema_version: Identifier,
    /// Whether a live veraPDF binary was executed.
    pub live_oracle_enabled: bool,
    /// Run metadata.
    pub metadata: OracleReportMetadata,
    /// Summary counters.
    pub summary: OracleCorpusSummary,
    /// Result rows.
    pub rows: Vec<OracleCorpusResultRow>,
}

impl OracleCorpusReport {
    /// Creates an oracle corpus report and derives summary counters from rows.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the schema id violates the identifier policy.
    pub fn new(
        schema_version: impl Into<String>,
        live_oracle_enabled: bool,
        metadata: OracleReportMetadata,
        rows: Vec<OracleCorpusResultRow>,
    ) -> Result<Self> {
        let summary = OracleCorpusSummary::from_rows(&rows);
        Ok(Self {
            schema_version: Identifier::new(schema_version)?,
            live_oracle_enabled,
            metadata,
            summary,
            rows,
        })
    }
}

/// Oracle corpus summary counters.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleCorpusSummary {
    /// Total rows.
    pub total_rows: u64,
    /// Matching rows.
    pub matches: u64,
    /// Mismatching rows.
    pub mismatches: u64,
    /// Mismatches with a non-`unexpectedDrift` classification.
    pub classified_mismatches: u64,
    /// Unexpected drift rows.
    pub unexpected_drift: u64,
    /// False-compliant rows.
    pub false_compliant: u64,
    /// Release-blocking rows.
    pub release_blocking: u64,
    /// Timeout rows.
    pub timeout_count: u64,
    /// Match percentage in basis points.
    pub match_percentage_basis_points: u32,
    /// Row counts by tier.
    pub by_tier: BTreeMap<String, u64>,
    /// Row counts by profile family.
    pub by_profile_family: BTreeMap<String, u64>,
    /// Row counts by classification.
    pub by_classification: BTreeMap<String, u64>,
    /// Row counts by veraPDF outcome.
    pub by_vera_pdf_outcome: BTreeMap<String, u64>,
    /// Row counts by pdfv outcome.
    pub by_pdfv_outcome: BTreeMap<String, u64>,
}

impl OracleCorpusSummary {
    /// Builds summary counters from result rows.
    #[must_use]
    pub fn from_rows(rows: &[OracleCorpusResultRow]) -> Self {
        let mut summary = Self {
            total_rows: usize_to_u64(rows.len()),
            ..Self::default()
        };
        for row in rows {
            summary.add_row(row);
        }
        summary.match_percentage_basis_points =
            percentage_basis_points(summary.matches, summary.total_rows);
        summary
    }

    fn add_row(&mut self, row: &OracleCorpusResultRow) {
        increment(&mut self.by_tier, tier_key(row.tier));
        increment(
            &mut self.by_profile_family,
            row.profile_family.as_str().to_owned(),
        );
        increment(
            &mut self.by_classification,
            classification_key(row.classification),
        );
        if let Some(outcome) = row.vera_pdf_outcome {
            increment(&mut self.by_vera_pdf_outcome, outcome_key(outcome));
        }
        if let Some(outcome) = row.pdfv_outcome {
            increment(&mut self.by_pdfv_outcome, outcome_key(outcome));
        }
        match row.classification {
            OracleDriftClassification::Match => self.matches = self.matches.saturating_add(1),
            OracleDriftClassification::UnexpectedDrift => {
                self.mismatches = self.mismatches.saturating_add(1);
                self.unexpected_drift = self.unexpected_drift.saturating_add(1);
            }
            _ => {
                self.mismatches = self.mismatches.saturating_add(1);
                self.classified_mismatches = self.classified_mismatches.saturating_add(1);
            }
        }
        if row.false_compliant {
            self.false_compliant = self.false_compliant.saturating_add(1);
        }
        if row.release_blocking {
            self.release_blocking = self.release_blocking.saturating_add(1);
        }
        if matches!(row.vera_pdf_status, OracleExecutionStatus::Timeout)
            || matches!(row.pdfv_status, OracleExecutionStatus::Timeout)
        {
            self.timeout_count = self.timeout_count.saturating_add(1);
        }
    }
}

/// Drift-only report derived from an oracle corpus report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OracleDriftReport {
    /// Report schema version.
    pub schema_version: Identifier,
    /// Summary counters copied from the corpus report.
    pub summary: OracleCorpusSummary,
    /// Non-matching or release-blocking rows.
    pub rows: Vec<OracleCorpusResultRow>,
}

/// Builds a drift-only report.
///
/// # Errors
///
/// Returns [`ConfigError`] if the schema id cannot be constructed.
pub fn oracle_drift_report(report: &OracleCorpusReport) -> Result<OracleDriftReport> {
    let rows = report
        .rows
        .iter()
        .filter(|row| {
            row.release_blocking || !matches!(row.classification, OracleDriftClassification::Match)
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(OracleDriftReport {
        schema_version: Identifier::new("phase22-oracle-drift-v1")?,
        summary: OracleCorpusSummary::from_rows(&rows),
        rows,
    })
}

/// Classifies an oracle observation according to the release-readiness rules.
#[must_use]
pub fn classify_oracle_observation(
    row: &OracleCorpusRow,
    observation: &OracleRowObservation,
) -> OracleDriftClassification {
    if observation.vera_pdf_outcome == observation.pdfv_outcome
        && observation.vera_pdf_outcome.is_some()
        && matches!(
            observation.vera_pdf_status,
            OracleExecutionStatus::Completed
        )
        && matches!(observation.pdfv_status, OracleExecutionStatus::Completed)
    {
        return OracleDriftClassification::Match;
    }
    if is_false_compliant(observation) {
        return row
            .expected_classification
            .filter(|classification| {
                matches!(
                    classification,
                    OracleDriftClassification::VeraPdfAmbiguity
                        | OracleDriftClassification::PdfvBug
                )
            })
            .unwrap_or(OracleDriftClassification::UnexpectedDrift);
    }
    if matches!(observation.pdfv_status, OracleExecutionStatus::Timeout)
        || matches!(observation.vera_pdf_status, OracleExecutionStatus::Timeout)
    {
        return OracleDriftClassification::UnexpectedDrift;
    }
    if let Some(classification) = row.expected_classification {
        return classification;
    }
    if observation.pdfv_unsupported_rules > 0
        && matches!(observation.pdfv_outcome, Some(CorpusOutcome::Incomplete))
    {
        return OracleDriftClassification::ExpectedDriftUnsupportedRule;
    }
    OracleDriftClassification::UnexpectedDrift
}

fn validate_row(
    row: &OracleCorpusRow,
    seen: &mut std::collections::BTreeSet<CorpusRowId>,
) -> Result<()> {
    if !seen.insert(row.id.clone()) {
        return Err(ConfigError::InvalidValue {
            field: "id",
            reason: BoundedText::unchecked("duplicate corpus row id"),
        }
        .into());
    }
    if row.expected_features.len() > MAX_EXPECTED_FEATURES {
        return Err(ConfigError::InvalidValue {
            field: "expectedFeatures",
            reason: BoundedText::unchecked("too many expected semantic families"),
        }
        .into());
    }
    if let Some(producer) = &row.producer
        && producer.as_str().len() > MAX_PRODUCER_BYTES
    {
        return Err(ConfigError::InvalidValue {
            field: "producer",
            reason: BoundedText::unchecked("producer exceeds byte limit"),
        }
        .into());
    }
    validate_classification_note(row)?;
    validate_tier_license(row)
}

fn validate_classification_note(row: &OracleCorpusRow) -> Result<()> {
    let Some(classification) = row.expected_classification else {
        return Ok(());
    };
    if matches!(classification, OracleDriftClassification::Match) {
        return Err(ConfigError::InvalidValue {
            field: "expectedClassification",
            reason: BoundedText::unchecked("manifest must not predeclare match rows"),
        }
        .into());
    }
    if row.classification_note.is_none() {
        return Err(ConfigError::InvalidValue {
            field: "classificationNote",
            reason: BoundedText::unchecked("known drift rows require a classification note"),
        }
        .into());
    }
    Ok(())
}

fn validate_tier_license(row: &OracleCorpusRow) -> Result<()> {
    if matches!(row.tier, CorpusTier::T3RealWorld)
        && !matches!(row.license, CorpusLicenseClass::Private)
    {
        return Err(ConfigError::InvalidValue {
            field: "license",
            reason: BoundedText::unchecked("T3 real-world rows must be private"),
        }
        .into());
    }
    if !matches!(row.tier, CorpusTier::T3RealWorld)
        && matches!(row.license, CorpusLicenseClass::Private)
    {
        return Err(ConfigError::InvalidValue {
            field: "license",
            reason: BoundedText::unchecked("private rows are only allowed in T3"),
        }
        .into());
    }
    Ok(())
}

fn result_reason(
    row: &OracleCorpusRow,
    observation: &OracleRowObservation,
    classification: OracleDriftClassification,
) -> Result<Option<BoundedText>> {
    if let Some(note) = &row.classification_note {
        return Ok(Some(note.clone()));
    }
    if matches!(classification, OracleDriftClassification::Match) {
        return Ok(None);
    }
    let reason = match (observation.vera_pdf_outcome, observation.pdfv_outcome) {
        (Some(vera_pdf), Some(pdfv)) => format!("veraPDF {vera_pdf:?}, pdfv {pdfv:?}"),
        (None, Some(pdfv)) => format!("veraPDF unavailable, pdfv {pdfv:?}"),
        (Some(vera_pdf), None) => format!("veraPDF {vera_pdf:?}, pdfv unavailable"),
        (None, None) => String::from("both validators failed before outcome classification"),
    };
    Ok(Some(BoundedText::new(reason, MAX_REASON_BYTES)?))
}

fn is_false_compliant(observation: &OracleRowObservation) -> bool {
    matches!(observation.pdfv_outcome, Some(CorpusOutcome::Valid))
        && matches!(observation.vera_pdf_outcome, Some(CorpusOutcome::Invalid))
}

fn verapdf_flavour_code(flavour: &ValidationFlavour) -> Option<String> {
    let family = flavour.family.as_str();
    let conformance = flavour.conformance.as_str();
    match family {
        "pdfa" => Some(format!("{}{conformance}", flavour.part)),
        "pdfua" if flavour.part.get() == 1 => Some(String::from("ua1")),
        "pdfua" if flavour.part.get() == 2 => Some(String::from("ua2")),
        "wtpdf" if conformance == "accessibility" => Some(String::from("wt1a")),
        "wtpdf" if conformance == "reuse" => Some(String::from("wt1r")),
        _ => None,
    }
}

fn percentage_basis_points(numerator: u64, denominator: u64) -> u32 {
    if denominator == 0 {
        return 0;
    }
    let basis_points = numerator.saturating_mul(10_000) / denominator;
    u32::try_from(basis_points).unwrap_or(u32::MAX)
}

fn increment(map: &mut BTreeMap<String, u64>, key: String) {
    let counter = map.entry(key).or_default();
    *counter = counter.saturating_add(1);
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn tier_key(tier: CorpusTier) -> String {
    match tier {
        CorpusTier::T0Generated => String::from("t0Generated"),
        CorpusTier::T1CheckedIn => String::from("t1CheckedIn"),
        CorpusTier::T2PublicConformance => String::from("t2PublicConformance"),
        CorpusTier::T3RealWorld => String::from("t3RealWorld"),
        CorpusTier::T4Adversarial => String::from("t4Adversarial"),
    }
}

fn classification_key(classification: OracleDriftClassification) -> String {
    match classification {
        OracleDriftClassification::Match => String::from("match"),
        OracleDriftClassification::ExpectedDriftUnsupportedRule => {
            String::from("expectedDriftUnsupportedRule")
        }
        OracleDriftClassification::ExpectedDriftScope => String::from("expectedDriftScope"),
        OracleDriftClassification::ExpectedDriftLimit => String::from("expectedDriftLimit"),
        OracleDriftClassification::VeraPdfAmbiguity => String::from("veraPdfAmbiguity"),
        OracleDriftClassification::UnexpectedDrift => String::from("unexpectedDrift"),
        OracleDriftClassification::PdfvBug => String::from("pdfvBug"),
    }
}

fn outcome_key(outcome: CorpusOutcome) -> String {
    match outcome {
        CorpusOutcome::Valid => String::from("valid"),
        CorpusOutcome::Invalid => String::from("invalid"),
        CorpusOutcome::Encrypted => String::from("encrypted"),
        CorpusOutcome::Incomplete => String::from("incomplete"),
        CorpusOutcome::ParseFailed => String::from("parseFailed"),
    }
}

impl fmt::Display for ValidationFlavour {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}-{}-{}",
            self.family, self.part, self.conformance
        )
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CorpusLicenseClass, CorpusPath, CorpusRowId, CorpusTier, EncryptionClass,
        OracleCorpusManifest, OracleCorpusResultRow, OracleCorpusRow, OracleDriftClassification,
        OracleExecutionStatus, OracleProfilePolicy, OracleRowObservation, SemanticFamilyName,
        SizeBucket, TaggingClass,
    };
    use crate::{BoundedText, CorpusOutcome, Identifier, ValidationFlavour};

    #[test]
    fn test_should_reject_parent_traversal_corpus_path() {
        let result = CorpusPath::new("../private.pdf");

        assert!(result.is_err());
    }

    #[test]
    fn test_should_classify_match_rows() -> crate::Result<()> {
        let row = base_row()?;
        let observation = OracleRowObservation {
            vera_pdf_outcome: Some(CorpusOutcome::Valid),
            pdfv_outcome: Some(CorpusOutcome::Valid),
            pdfv_unsupported_rules: 0,
            vera_pdf_status: OracleExecutionStatus::Completed,
            pdfv_status: OracleExecutionStatus::Completed,
        };

        let result = OracleCorpusResultRow::from_observation(&row, observation)?;

        assert_eq!(result.classification, OracleDriftClassification::Match);
        assert!(!result.release_blocking);
        Ok(())
    }

    #[test]
    fn test_should_keep_false_compliant_rows_unexpected_without_ambiguity() -> crate::Result<()> {
        let mut row = base_row()?;
        row.expected_classification = Some(OracleDriftClassification::ExpectedDriftScope);
        row.classification_note = Some(BoundedText::new("known scope gap", 64)?);
        let observation = OracleRowObservation {
            vera_pdf_outcome: Some(CorpusOutcome::Invalid),
            pdfv_outcome: Some(CorpusOutcome::Valid),
            pdfv_unsupported_rules: 0,
            vera_pdf_status: OracleExecutionStatus::Completed,
            pdfv_status: OracleExecutionStatus::Completed,
        };

        let result = OracleCorpusResultRow::from_observation(&row, observation)?;

        assert_eq!(
            result.classification,
            OracleDriftClassification::UnexpectedDrift
        );
        assert!(result.false_compliant);
        assert!(result.release_blocking);
        Ok(())
    }

    #[test]
    fn test_should_validate_manifest_duplicate_ids() -> crate::Result<()> {
        let row = base_row()?;
        let manifest = OracleCorpusManifest {
            schema_version: Identifier::new("test")?,
            corpus_root: CorpusPath::new(".")?,
            rows: vec![row.clone(), row],
        };

        let result = manifest.validate();

        assert!(result.is_err());
        Ok(())
    }

    fn base_row() -> crate::Result<OracleCorpusRow> {
        Ok(OracleCorpusRow {
            id: CorpusRowId::new("row-1")?,
            path: CorpusPath::new("minimal-valid.pdf")?,
            tier: CorpusTier::T2PublicConformance,
            profile_policy: OracleProfilePolicy::Explicit {
                flavour: ValidationFlavour::new("pdfa", std::num::NonZeroU32::MIN, "b")?,
            },
            expected_features: vec![SemanticFamilyName::new("parser")?],
            producer: None,
            size_bucket: SizeBucket::Tiny,
            encryption: EncryptionClass::Unencrypted,
            tagging: TaggingClass::Untagged,
            license: CorpusLicenseClass::ProjectFixture,
            expected_classification: None,
            classification_note: None,
        })
    }
}
