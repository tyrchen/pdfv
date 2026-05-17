//! Writes generated-profile model-schema parity metrics.

use std::{fs, path::Path};

use pdfv_core::{ReportError, Result, model_schema_parity_report};

fn main() -> Result<()> {
    let report = model_schema_parity_report()?;
    let output_dir = Path::new("target/parity");
    #[allow(
        clippy::disallowed_methods,
        reason = "Makefile parity target is a synchronous one-shot local artifact writer"
    )]
    fs::create_dir_all(output_dir).map_err(|source| pdfv_core::PdfvError::Io {
        path: Some(output_dir.to_path_buf()),
        source,
    })?;
    let output_path = output_dir.join("model-schema.json");
    #[allow(
        clippy::disallowed_types,
        reason = "Makefile parity target writes one local JSON artifact synchronously"
    )]
    let file = fs::File::create(&output_path).map_err(|source| pdfv_core::PdfvError::Io {
        path: Some(output_path.clone()),
        source,
    })?;
    serde_json::to_writer_pretty(file, &report).map_err(ReportError::from)?;
    Ok(())
}
