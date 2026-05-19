//! Rebuilds live oracle summary and drift artifacts from `oracle-corpus.json`.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{env, fs, num::NonZeroUsize, path::Path};

use pdfv_core::{
    BoundedText, ConfigError, CorpusTier, OracleCorpusReport, OracleDriftClassification, PdfvError,
    ReportError, Result, oracle_drift_report,
};

const OUTPUT_DIR: &str = "target/parity";
const RELEASE_GATE_ENV: &str = "PDFV_ORACLE_REQUIRE_T3_RELEASE";
const MIN_T3_ROWS_ENV: &str = "PDFV_ORACLE_MIN_T3_ROWS";
const DEFAULT_MIN_T3_ROWS: usize = 1_000;
const MIN_T3_MATCH_BASIS_POINTS: u64 = 9_500;

fn main() -> Result<()> {
    let output_dir = Path::new(OUTPUT_DIR);
    let corpus_path = output_dir.join("oracle-corpus.json");
    let report = read_report(&corpus_path)?;
    let normalized = OracleCorpusReport::new(
        report.schema_version.as_str(),
        report.live_oracle_enabled,
        report.metadata.clone(),
        report.rows.clone(),
    )?;
    write_json_file(&output_dir.join("oracle-summary.json"), &normalized.summary)?;
    write_json_file(
        &output_dir.join("oracle-drift.json"),
        &oracle_drift_report(&normalized)?,
    )?;
    ensure_oracle_gates(&normalized, OracleGateConfig::from_env()?)
}

fn read_report(path: &Path) -> Result<OracleCorpusReport> {
    let file = fs::File::open(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    serde_json::from_reader(file)
        .map_err(ReportError::from)
        .map_err(Into::into)
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = fs::File::create(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    serde_json::to_writer_pretty(file, value).map_err(ReportError::from)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OracleGateConfig {
    require_t3_release: bool,
    min_t3_rows: NonZeroUsize,
}

impl OracleGateConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            require_t3_release: env_flag(RELEASE_GATE_ENV)?,
            min_t3_rows: env_min_t3_rows()?,
        })
    }

    #[cfg(test)]
    fn public_tier_gate() -> Self {
        Self {
            require_t3_release: false,
            min_t3_rows: NonZeroUsize::new(DEFAULT_MIN_T3_ROWS).unwrap_or(NonZeroUsize::MIN),
        }
    }

    #[cfg(test)]
    fn release_gate_with_min_t3_rows(min_t3_rows: NonZeroUsize) -> Self {
        Self {
            require_t3_release: true,
            min_t3_rows,
        }
    }
}

fn ensure_oracle_gates(report: &OracleCorpusReport, config: OracleGateConfig) -> Result<()> {
    if report.summary.false_compliant > 0 {
        return Err(invalid_value(
            "oracleCorpus",
            "oracle corpus has false-compliant rows",
        ));
    }
    if report.summary.unexpected_drift > 0 {
        return Err(invalid_value(
            "oracleCorpus",
            "oracle corpus has unexpected drift",
        ));
    }
    let t3 = report
        .rows
        .iter()
        .filter(|row| matches!(row.tier, CorpusTier::T3RealWorld))
        .collect::<Vec<_>>();
    if config.require_t3_release && t3.len() < config.min_t3_rows.get() {
        return Err(invalid_value(
            "oracleCorpus",
            format!(
                "T3 release corpus has {} rows; {} required",
                t3.len(),
                config.min_t3_rows,
            ),
        ));
    }
    if !t3.is_empty() {
        let matches = t3
            .iter()
            .filter(|row| matches!(row.classification, OracleDriftClassification::Match))
            .count();
        let basis_points = percentage_basis_points(matches, t3.len());
        if basis_points < MIN_T3_MATCH_BASIS_POINTS {
            return Err(invalid_value("oracleCorpus", "T3 match rate is below 95%"));
        }
    }
    Ok(())
}

fn env_flag(name: &'static str) -> Result<bool> {
    match env::var(name) {
        Ok(value) => match value.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" => Ok(true),
            "0" | "false" | "FALSE" | "no" | "NO" => Ok(false),
            _ => Err(invalid_value(
                name,
                "expected one of 1, true, yes, 0, false, or no",
            )),
        },
        Err(env::VarError::NotPresent) => Ok(false),
        Err(source) => Err(invalid_value(
            name,
            format!("invalid environment value: {source}"),
        )),
    }
}

fn env_min_t3_rows() -> Result<NonZeroUsize> {
    match env::var(MIN_T3_ROWS_ENV) {
        Ok(value) => parse_non_zero_usize(MIN_T3_ROWS_ENV, &value),
        Err(env::VarError::NotPresent) => {
            Ok(NonZeroUsize::new(DEFAULT_MIN_T3_ROWS).unwrap_or(NonZeroUsize::MIN))
        }
        Err(source) => Err(invalid_value(
            MIN_T3_ROWS_ENV,
            format!("invalid environment value: {source}"),
        )),
    }
}

fn parse_non_zero_usize(field: &'static str, value: &str) -> Result<NonZeroUsize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|source| invalid_value(field, format!("invalid integer value: {source}")))?;
    NonZeroUsize::new(parsed).ok_or_else(|| invalid_value(field, "value must be at least 1"))
}

fn percentage_basis_points(matches: usize, total: usize) -> u64 {
    if total == 0 {
        return 0;
    }
    let matches = u64::try_from(matches).unwrap_or(u64::MAX);
    let total = u64::try_from(total).unwrap_or(u64::MAX);
    matches.saturating_mul(10_000) / total
}

fn invalid_value(field: &'static str, reason: impl Into<String>) -> PdfvError {
    match BoundedText::new(reason, 1024) {
        Ok(reason) => ConfigError::InvalidValue { field, reason }.into(),
        Err(error) => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use pdfv_core::{OracleCorpusReport, OracleDriftClassification, ReportError};
    use serde_json::json;

    use super::{OracleGateConfig, ensure_oracle_gates, parse_non_zero_usize};

    #[test]
    fn test_should_reject_unexpected_drift_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::UnexpectedDrift)?;

        let result = ensure_oracle_gates(&report, OracleGateConfig::public_tier_gate());

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_accept_match_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::Match)?;

        ensure_oracle_gates(&report, OracleGateConfig::public_tier_gate())
    }

    #[test]
    fn test_should_reject_release_gate_without_t3_rows() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::Match)?;

        let result = ensure_oracle_gates(
            &report,
            OracleGateConfig::release_gate_with_min_t3_rows(non_zero(1)),
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_release_gate_below_t3_row_threshold() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[("t3RealWorld", OracleDriftClassification::Match)])?;

        let result = ensure_oracle_gates(
            &report,
            OracleGateConfig::release_gate_with_min_t3_rows(non_zero(2)),
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_release_gate_below_t3_match_threshold() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[
            ("t3RealWorld", OracleDriftClassification::Match),
            (
                "t3RealWorld",
                OracleDriftClassification::ExpectedDriftUnsupportedRule,
            ),
        ])?;

        let result = ensure_oracle_gates(
            &report,
            OracleGateConfig::release_gate_with_min_t3_rows(non_zero(2)),
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_accept_release_gate_with_enough_t3_matches() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[
            ("t3RealWorld", OracleDriftClassification::Match),
            ("t3RealWorld", OracleDriftClassification::Match),
        ])?;

        ensure_oracle_gates(
            &report,
            OracleGateConfig::release_gate_with_min_t3_rows(non_zero(2)),
        )
    }

    #[test]
    fn test_should_reject_zero_min_t3_rows_env_value() {
        let result = parse_non_zero_usize("PDFV_ORACLE_MIN_T3_ROWS", "0");

        assert!(result.is_err());
    }

    fn report_with_classification(
        classification: OracleDriftClassification,
    ) -> pdfv_core::Result<OracleCorpusReport> {
        report_with_rows(&[("t2PublicConformance", classification)])
    }

    fn report_with_rows(
        rows: &[(&'static str, OracleDriftClassification)],
    ) -> pdfv_core::Result<OracleCorpusReport> {
        let json_rows = rows
            .iter()
            .enumerate()
            .map(|(index, (tier, classification))| {
                let classification_text = classification_text(*classification);
                let release_blocking =
                    matches!(classification, OracleDriftClassification::UnexpectedDrift);
                json!({
                    "id": format!("row-{index}"),
                    "path": "tests/fixtures/minimal-valid.pdf",
                    "tier": tier,
                    "profilePolicy": {"kind": "autoDefaultPdfa1b"},
                    "profileFamily": "auto",
                    "expectedFeatures": ["parser"],
                    "sizeBucket": "tiny",
                    "encryption": "unencrypted",
                    "tagging": "untagged",
                    "license": if *tier == "t3RealWorld" { "private" } else { "projectFixture" },
                    "veraPdfOutcome": "valid",
                    "pdfvOutcome": if *classification == OracleDriftClassification::Match { "valid" } else { "incomplete" },
                    "veraPdfStatus": "completed",
                    "pdfvStatus": "completed",
                    "classification": classification_text,
                    "falseCompliant": false,
                    "releaseBlocking": release_blocking,
                    "pdfvUnsupportedRules": if *classification == OracleDriftClassification::Match { 0 } else { 1 }
                })
            })
            .collect::<Vec<_>>();
        serde_json::from_value(json!({
            "schemaVersion": "test",
            "liveOracleEnabled": true,
            "metadata": {
                "pdfvVersion": "test",
                "pdfvCommit": "test",
                "veraPdfVersion": "test",
                "javaVersion": "test",
                "os": "test",
                "arch": "test",
                "vendorPins": {}
            },
            "summary": {
                "totalRows": rows.len(),
                "matches": rows.iter().filter(|(_, classification)| *classification == OracleDriftClassification::Match).count(),
                "mismatches": rows.iter().filter(|(_, classification)| *classification != OracleDriftClassification::Match).count(),
                "classifiedMismatches": 0,
                "unexpectedDrift": rows.iter().filter(|(_, classification)| *classification == OracleDriftClassification::UnexpectedDrift).count(),
                "falseCompliant": 0,
                "releaseBlocking": rows.iter().filter(|(_, classification)| *classification == OracleDriftClassification::UnexpectedDrift).count(),
                "timeoutCount": 0,
                "matchPercentageBasisPoints": 10000,
                "byTier": {"t2PublicConformance": 1},
                "byProfileFamily": {"auto": 1},
                "byClassification": {},
                "byVeraPdfOutcome": {"valid": 1},
                "byPdfvOutcome": {"valid": 1}
            },
            "rows": json_rows
        }))
        .map_err(ReportError::from)
        .map_err(Into::into)
    }

    fn classification_text(classification: OracleDriftClassification) -> &'static str {
        let classification_text = match classification {
            OracleDriftClassification::Match => "match",
            OracleDriftClassification::UnexpectedDrift => "unexpectedDrift",
            OracleDriftClassification::ExpectedDriftUnsupportedRule => {
                "expectedDriftUnsupportedRule"
            }
            OracleDriftClassification::ExpectedDriftScope => "expectedDriftScope",
            OracleDriftClassification::ExpectedDriftLimit => "expectedDriftLimit",
            OracleDriftClassification::VeraPdfAmbiguity => "veraPdfAmbiguity",
            OracleDriftClassification::PdfvBug => "pdfvBug",
            _ => "expectedDriftScope",
        };
        classification_text
    }

    fn non_zero(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
    }
}
