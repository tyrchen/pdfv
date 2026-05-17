//! Writes Java-free semantic corpus agreement parity metrics.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile parity targets are synchronous one-shot local artifact writers"
)]

use std::{fs, path::Path};

use pdfv_core::{
    BoundedText, ConfigError, CorpusAgreementReport, PdfvError, ReportError, Result,
    corpus_agreement_report,
};

const OUTPUT_DIR: &str = "target/parity";

fn main() -> Result<()> {
    let report = corpus_agreement_report()?;
    let output_dir = Path::new(OUTPUT_DIR);
    fs::create_dir_all(output_dir).map_err(|source| PdfvError::Io {
        path: Some(output_dir.to_path_buf()),
        source,
    })?;
    let output_path = output_dir.join("corpus-agreement.json");
    let file = fs::File::create(&output_path).map_err(|source| PdfvError::Io {
        path: Some(output_path),
        source,
    })?;
    serde_json::to_writer_pretty(file, &report).map_err(ReportError::from)?;
    ensure_no_unexpected_drift(&report)?;
    Ok(())
}

fn ensure_no_unexpected_drift(report: &CorpusAgreementReport) -> Result<()> {
    if report.summary.unexpected_drift == 0 {
        return Ok(());
    }
    Err(ConfigError::InvalidValue {
        field: "corpusAgreement",
        reason: BoundedText::new(
            format!(
                "{} unexpected corpus agreement drift rows",
                report.summary.unexpected_drift
            ),
            256,
        )?,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use pdfv_core::CorpusAgreementReport;
    use serde_json::json;

    use super::ensure_no_unexpected_drift;

    #[test]
    fn test_should_fail_when_corpus_report_has_unexpected_drift() -> pdfv_core::Result<()> {
        let report = report_with_unexpected_drift(1)?;

        let result = ensure_no_unexpected_drift(&report);

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_should_pass_when_corpus_report_has_no_unexpected_drift() -> pdfv_core::Result<()> {
        let report = report_with_unexpected_drift(0)?;

        ensure_no_unexpected_drift(&report)
    }

    fn report_with_unexpected_drift(
        unexpected_drift: u64,
    ) -> pdfv_core::Result<CorpusAgreementReport> {
        serde_json::from_value(json!({
            "schemaVersion": "test",
            "liveOracleEnabled": false,
            "summary": {
                "totalRows": unexpected_drift,
                "matches": 0,
                "expectedDrift": 0,
                "unexpectedDrift": unexpected_drift,
            },
            "rows": [],
        }))
        .map_err(pdfv_core::ReportError::from)
        .map_err(Into::into)
    }
}
