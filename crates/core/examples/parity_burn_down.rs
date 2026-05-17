//! Writes unsupported-rule cluster and burn-down parity metrics.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile parity targets are synchronous one-shot local artifact writers"
)]

use std::{env, fs, path::Path};

use pdfv_core::{
    PdfvError, ReportError, Result, UnsupportedRuleClusterReport,
    rule_burn_down_report_from_clusters, unsupported_rule_cluster_report,
};

const BASELINE_ENV: &str = "PDFV_PARITY_BASELINE_DIR";
const OUTPUT_DIR: &str = "target/parity";

fn main() -> Result<()> {
    let current = unsupported_rule_cluster_report()?;
    let previous = read_baseline_clusters()?;
    let burn_down = rule_burn_down_report_from_clusters(&current, previous.as_ref())?;

    let output_dir = Path::new(OUTPUT_DIR);
    fs::create_dir_all(output_dir).map_err(|source| PdfvError::Io {
        path: Some(output_dir.to_path_buf()),
        source,
    })?;
    write_json_file(&output_dir.join("unsupported-rule-clusters.json"), &current)?;
    write_json_file(&output_dir.join("rule-burn-down.json"), &burn_down)?;
    Ok(())
}

fn read_baseline_clusters() -> Result<Option<UnsupportedRuleClusterReport>> {
    let Some(baseline_dir) = env::var_os(BASELINE_ENV) else {
        return Ok(None);
    };
    let path = Path::new(&baseline_dir).join("unsupported-rule-clusters.json");
    if !path.exists() {
        return Ok(None);
    }
    let file = fs::File::open(&path).map_err(|source| PdfvError::Io {
        path: Some(path.clone()),
        source,
    })?;
    serde_json::from_reader(file)
        .map(Some)
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
