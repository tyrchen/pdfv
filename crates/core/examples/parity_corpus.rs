//! Writes Java-free semantic corpus agreement parity metrics.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile parity targets are synchronous one-shot local artifact writers"
)]

use std::{fs, path::Path};

use pdfv_core::{PdfvError, ReportError, Result, corpus_agreement_report};

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
    Ok(())
}
