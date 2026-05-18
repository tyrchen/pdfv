//! Rebuilds live oracle summary and drift artifacts from `oracle-corpus.json`.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{fs, path::Path};

use pdfv_core::{
    BoundedText, ConfigError, CorpusTier, OracleCorpusReport, OracleDriftClassification, PdfvError,
    ReportError, Result, oracle_drift_report,
};

const OUTPUT_DIR: &str = "target/parity";

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
    ensure_oracle_gates(&normalized)
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

fn ensure_oracle_gates(report: &OracleCorpusReport) -> Result<()> {
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
    if !t3.is_empty() {
        let matches = t3
            .iter()
            .filter(|row| matches!(row.classification, OracleDriftClassification::Match))
            .count();
        let basis_points = u64::try_from(matches)
            .unwrap_or(u64::MAX)
            .saturating_mul(10_000)
            / u64::try_from(t3.len()).unwrap_or(u64::MAX);
        if basis_points < 9_500 {
            return Err(invalid_value("oracleCorpus", "T3 match rate is below 95%"));
        }
    }
    Ok(())
}

fn invalid_value(field: &'static str, reason: impl Into<String>) -> PdfvError {
    match BoundedText::new(reason, 1024) {
        Ok(reason) => ConfigError::InvalidValue { field, reason }.into(),
        Err(error) => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use pdfv_core::{OracleCorpusReport, OracleDriftClassification, ReportError};
    use serde_json::json;

    use super::ensure_oracle_gates;

    #[test]
    fn test_should_reject_unexpected_drift_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::UnexpectedDrift)?;

        let result = ensure_oracle_gates(&report);

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_accept_match_summary() -> pdfv_core::Result<()> {
        let report = report_with_classification(OracleDriftClassification::Match)?;

        ensure_oracle_gates(&report)
    }

    fn report_with_classification(
        classification: OracleDriftClassification,
    ) -> pdfv_core::Result<OracleCorpusReport> {
        let classification_text = match classification {
            OracleDriftClassification::Match => "match",
            OracleDriftClassification::UnexpectedDrift => "unexpectedDrift",
            _ => "expectedDriftScope",
        };
        let release_blocking = matches!(classification, OracleDriftClassification::UnexpectedDrift);
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
                "totalRows": 1,
                "matches": if classification == OracleDriftClassification::Match { 1 } else { 0 },
                "mismatches": if classification == OracleDriftClassification::Match { 0 } else { 1 },
                "classifiedMismatches": 0,
                "unexpectedDrift": if release_blocking { 1 } else { 0 },
                "falseCompliant": 0,
                "releaseBlocking": if release_blocking { 1 } else { 0 },
                "timeoutCount": 0,
                "matchPercentageBasisPoints": if classification == OracleDriftClassification::Match { 10000 } else { 0 },
                "byTier": {"t2PublicConformance": 1},
                "byProfileFamily": {"auto": 1},
                "byClassification": {
                    "match": if classification == OracleDriftClassification::Match { 1 } else { 0 },
                    "unexpectedDrift": if release_blocking { 1 } else { 0 }
                },
                "byVeraPdfOutcome": {"valid": 1},
                "byPdfvOutcome": {"valid": 1}
            },
            "rows": [{
                "id": "row-1",
                "path": "tests/fixtures/minimal-valid.pdf",
                "tier": "t2PublicConformance",
                "profilePolicy": {"kind": "autoDefaultPdfa1b"},
                "profileFamily": "auto",
                "expectedFeatures": ["parser"],
                "sizeBucket": "tiny",
                "encryption": "unencrypted",
                "tagging": "untagged",
                "license": "projectFixture",
                "veraPdfOutcome": "valid",
                "pdfvOutcome": "valid",
                "veraPdfStatus": "completed",
                "pdfvStatus": "completed",
                "classification": classification_text,
                "falseCompliant": false,
                "releaseBlocking": release_blocking,
                "pdfvUnsupportedRules": 0
            }]
        }))
        .map_err(ReportError::from)
        .map_err(Into::into)
    }
}
