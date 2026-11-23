//! Gate checks shared by live oracle runs and saved-report verification.

use std::{collections::BTreeSet, env, num::NonZeroUsize, path::PathBuf};

use pdfv_core::{
    BoundedText, ConfigError, CorpusOutcome, CorpusTier, OracleCorpusReport, OracleCorpusResultRow,
    OracleDriftClassification, OracleExecutionStatus, PdfvError, Result,
};

const MIN_T3_MATCH_PERCENT: usize = 95;
pub(super) const COMPARABLE_REPORT_SCHEMA: &str = "phase22-oracle-corpus-v2";

pub(super) fn oracle_output_dir() -> PathBuf {
    env::var_os("PDFV_ORACLE_OUTPUT_DIR")
        .map_or_else(|| PathBuf::from("target/parity"), PathBuf::from)
}

pub(super) fn ensure_oracle_gates(
    report: &OracleCorpusReport,
    min_t3_documents: Option<NonZeroUsize>,
) -> Result<()> {
    if !report.live_oracle_enabled || report.rows.is_empty() {
        return Err(gate_error("oracle gates require nonempty live evidence"));
    }
    if min_t3_documents.is_some() && report.schema_version.as_str() != COMPARABLE_REPORT_SCHEMA {
        return Err(gate_error(
            "T3 evidence predates live profile-set verification",
        ));
    }
    let mut documents = BTreeSet::new();
    let mut matches = 0_usize;
    for row in &report.rows {
        check_row(row)?;
        if row.tier != CorpusTier::T3RealWorld
            || !matches!(row.profile_family.as_str(), "pdfa" | "pdfua" | "pdfa-pdfua")
        {
            continue;
        }
        if !documents.insert(&row.path) {
            return Err(gate_error("T3 evidence contains repeated document paths"));
        }
        if row.classification == OracleDriftClassification::Match {
            matches = matches.saturating_add(1);
        }
    }
    let total = documents.len();
    if let Some(minimum) = min_t3_documents
        && total < minimum.get()
    {
        return Err(gate_error(format!(
            "T3 release corpus has {total} distinct in-scope document paths; {minimum} required",
        )));
    }
    if matches.saturating_mul(100) < total.saturating_mul(MIN_T3_MATCH_PERCENT) {
        return Err(gate_error(format!(
            "T3 match rate is below 95% ({matches}/{total} rows)"
        )));
    }
    Ok(())
}

fn check_row(row: &OracleCorpusResultRow) -> Result<()> {
    let (Some(vera_pdf), Some(pdfv)) = (row.vera_pdf_outcome, row.pdfv_outcome) else {
        return Err(gate_error(format!(
            "row {} lacks completed outcomes",
            row.id
        )));
    };
    if row.vera_pdf_status != OracleExecutionStatus::Completed
        || row.pdfv_status != OracleExecutionStatus::Completed
    {
        return Err(gate_error(format!(
            "row {} lacks comparable executions",
            row.id
        )));
    }
    if row.false_compliant || (vera_pdf == CorpusOutcome::Invalid && pdfv == CorpusOutcome::Valid) {
        return Err(gate_error(format!("row {} is false-compliant", row.id)));
    }
    if row.release_blocking
        || matches!(
            row.classification,
            OracleDriftClassification::UnexpectedDrift | OracleDriftClassification::PdfvBug
        )
    {
        return Err(gate_error(format!(
            "row {} has release-blocking drift",
            row.id
        )));
    }
    if (row.classification == OracleDriftClassification::Match) != (vera_pdf == pdfv)
        || (row.classification == OracleDriftClassification::ExpectedDriftUnsupportedRule
            && !(matches!(vera_pdf, CorpusOutcome::Valid | CorpusOutcome::Invalid)
                && pdfv == CorpusOutcome::Incomplete
                && row.pdfv_unsupported_rules > 0))
        || (vera_pdf != pdfv
            && row
                .reason
                .as_ref()
                .is_none_or(|reason| reason.as_str().trim().is_empty()))
    {
        return Err(gate_error(format!(
            "row {} has inconsistent drift evidence",
            row.id
        )));
    }
    Ok(())
}

fn gate_error(reason: impl Into<String>) -> PdfvError {
    match BoundedText::new(reason, 1024) {
        Ok(reason) => ConfigError::InvalidValue {
            field: "oracleCorpus",
            reason,
        }
        .into(),
        Err(error) => error.into(),
    }
}
