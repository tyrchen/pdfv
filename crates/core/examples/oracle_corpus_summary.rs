//! Rebuilds live oracle summary and drift artifacts from `oracle-corpus.json`.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{env, fs, num::NonZeroUsize, path::Path};

use pdfv_core::{
    BoundedText, ConfigError, OracleCorpusReport, PdfvError, ReportError, Result,
    oracle_drift_report,
};

#[path = "support/oracle_gates.rs"]
mod oracle_gates;

use oracle_gates::{ensure_oracle_gates, oracle_output_dir};

const RELEASE_GATE_ENV: &str = "PDFV_ORACLE_REQUIRE_T3_RELEASE";
const MIN_T3_DOCUMENTS: usize = 1_000;

fn main() -> Result<()> {
    let output_dir = oracle_output_dir();
    let corpus_path = output_dir.join("oracle-corpus.json");
    let report = read_report(&corpus_path)?;
    let normalized = OracleCorpusReport::new(
        report.schema_version.as_str(),
        report.live_oracle_enabled,
        report.metadata,
        report.rows,
    )?;
    write_json_file(&output_dir.join("oracle-summary.json"), &normalized.summary)?;
    write_json_file(
        &output_dir.join("oracle-drift.json"),
        &oracle_drift_report(&normalized)?,
    )?;
    ensure_oracle_gates(&normalized, release_minimum_from_env()?)
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

fn release_minimum_from_env() -> Result<Option<NonZeroUsize>> {
    if env_flag(RELEASE_GATE_ENV)? {
        NonZeroUsize::new(MIN_T3_DOCUMENTS)
            .map(Some)
            .ok_or_else(|| invalid_value("oracleCorpus", "release minimum must be positive"))
    } else {
        Ok(None)
    }
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

fn invalid_value(field: &'static str, reason: impl Into<String>) -> PdfvError {
    match BoundedText::new(reason, 1024) {
        Ok(reason) => ConfigError::InvalidValue { field, reason }.into(),
        Err(error) => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use pdfv_core::{
        CorpusOutcome, Identifier, OracleCorpusReport, OracleDriftClassification,
        OracleExecutionStatus, ReportError,
    };
    use serde_json::json;

    use super::{ensure_oracle_gates, oracle_gates::COMPARABLE_REPORT_SCHEMA};

    #[test]
    fn test_should_reject_unexpected_drift_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::UnexpectedDrift)?;

        let result = ensure_oracle_gates(&report, None);

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_accept_match_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::Match)?;

        ensure_oracle_gates(&report, None)
    }

    #[test]
    fn test_should_reject_release_gate_without_t3_rows() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::Match)?;

        let result = ensure_oracle_gates(&report, NonZeroUsize::new(1));

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_release_gate_below_t3_row_threshold() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[("t3RealWorld", OracleDriftClassification::Match)])?;

        let result = ensure_oracle_gates(&report, NonZeroUsize::new(2));

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

        let result = ensure_oracle_gates(&report, NonZeroUsize::new(2));

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_accept_release_gate_with_enough_t3_matches() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[
            ("t3RealWorld", OracleDriftClassification::Match),
            ("t3RealWorld", OracleDriftClassification::Match),
        ])?;

        ensure_oracle_gates(&report, NonZeroUsize::new(2))
    }

    #[test]
    fn test_should_reject_release_gate_with_repeated_document_paths() -> pdfv_core::Result<()> {
        let report = report_with_rows(&[
            ("t3RealWorld", OracleDriftClassification::Match),
            ("t3RealWorld", OracleDriftClassification::Match),
        ])?;
        let mut rows = report.rows;
        for row in &mut rows {
            row.path = pdfv_core::CorpusPath::new("repeated.pdf")?;
        }
        let report =
            OracleCorpusReport::new(COMPARABLE_REPORT_SCHEMA, true, report.metadata, rows)?;

        assert!(ensure_oracle_gates(&report, NonZeroUsize::new(2)).is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_classified_pdfv_bug() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::PdfvBug)?;

        assert!(ensure_oracle_gates(&report, None).is_err());
        Ok(())
    }

    #[test]
    fn test_should_recheck_saved_observations() -> pdfv_core::Result<()> {
        let mut report = report_with_classification(OracleDriftClassification::Match)?;
        if let Some(row) = report.rows.first_mut() {
            row.pdfv_status = OracleExecutionStatus::Failed;
        }
        assert!(ensure_oracle_gates(&report, None).is_err());

        let mut report = report_with_classification(OracleDriftClassification::Match)?;
        if let Some(row) = report.rows.first_mut() {
            row.vera_pdf_outcome = Some(CorpusOutcome::Invalid);
        }
        assert!(ensure_oracle_gates(&report, None).is_err());
        Ok(())
    }

    #[test]
    fn test_should_exclude_out_of_scope_documents_from_release_count() -> pdfv_core::Result<()> {
        let mut report = report_with_rows(&[("t3RealWorld", OracleDriftClassification::Match)])?;
        if let Some(row) = report.rows.first_mut() {
            row.profile_family = Identifier::new("wtpdf")?;
        }

        assert!(ensure_oracle_gates(&report, NonZeroUsize::new(1)).is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_legacy_reports_for_the_strict_release_gate() -> pdfv_core::Result<()> {
        let mut report = report_with_rows(&[("t3RealWorld", OracleDriftClassification::Match)])?;
        report.schema_version = Identifier::new("phase22-oracle-corpus-v1")?;

        ensure_oracle_gates(&report, None)?;
        assert!(ensure_oracle_gates(&report, NonZeroUsize::new(1)).is_err());
        Ok(())
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
                    matches!(classification, OracleDriftClassification::UnexpectedDrift | OracleDriftClassification::PdfvBug);
                json!({
                    "id": format!("row-{index}"),
                    "path": format!("fixtures/doc-{index}.pdf"),
                    "tier": tier,
                    "profilePolicy": {"kind": "autoDefaultPdfa1b"},
                    "profileFamily": "pdfa",
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
                    "reason": "test review note",
                    "pdfvUnsupportedRules": if *classification == OracleDriftClassification::Match { 0 } else { 1 }
                })
            })
            .collect::<Vec<_>>();
        let report: OracleCorpusReport = serde_json::from_value(json!({
            "schemaVersion": COMPARABLE_REPORT_SCHEMA,
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
        .map_err(ReportError::from)?;
        OracleCorpusReport::new(
            COMPARABLE_REPORT_SCHEMA,
            report.live_oracle_enabled,
            report.metadata,
            report.rows,
        )
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
}
