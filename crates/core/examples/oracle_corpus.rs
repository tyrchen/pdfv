//! Runs the live veraPDF oracle corpus and writes release-readiness artifacts.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{
    collections::BTreeMap,
    env, fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use pdfv_core::{
    BoundedText, ConfigError, CorpusOutcome, CorpusTier, ENGINE_VERSION, OracleCorpusManifest,
    OracleCorpusReport, OracleCorpusResultRow, OracleDriftClassification, OracleExecutionStatus,
    OracleReportMetadata, OracleRowObservation, PdfvError, ReportError, Result, Validator,
    oracle_drift_report,
};

const MANIFEST_ENV: &str = "PDFV_ORACLE_CORPUS_MANIFEST";
const CORPUS_ROOT_ENV: &str = "PDFV_ORACLE_CORPUS_ROOT";
const VERAPDF_BIN_ENV: &str = "PDFV_VERAPDF_BIN";
const TIMEOUT_SECONDS_ENV: &str = "PDFV_ORACLE_TIMEOUT_SECONDS";
const MAX_OUTPUT_BYTES_ENV: &str = "PDFV_ORACLE_MAX_OUTPUT_BYTES";
const OUTPUT_DIR: &str = "target/parity";
const DEFAULT_VERAPDF_BIN: &str = "verapdf";
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

fn main() -> Result<()> {
    let manifest_path = required_path_env(MANIFEST_ENV)?;
    let manifest = read_manifest(&manifest_path)?;
    manifest.validate()?;
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| invalid_value("manifestPath", "manifest must have a parent directory"))?;
    let corpus_root = corpus_root(manifest_dir, &manifest)?;
    let vera_pdf_bin =
        env::var(VERAPDF_BIN_ENV).unwrap_or_else(|_| String::from(DEFAULT_VERAPDF_BIN));
    let timeout = Duration::from_secs(env_u64(TIMEOUT_SECONDS_ENV, DEFAULT_TIMEOUT_SECONDS)?);
    let max_output_bytes = env_u64(MAX_OUTPUT_BYTES_ENV, DEFAULT_MAX_OUTPUT_BYTES)?;

    let metadata = report_metadata(&vera_pdf_bin, timeout, max_output_bytes)?;
    let rows = manifest
        .rows
        .iter()
        .map(|row| run_row(row, &corpus_root, &vera_pdf_bin, timeout, max_output_bytes))
        .collect::<Result<Vec<_>>>()?;
    let report = OracleCorpusReport::new("phase22-oracle-corpus-v1", true, metadata, rows)?;

    write_artifacts(&report)?;
    ensure_oracle_gates(&report)
}

fn read_manifest(path: &Path) -> Result<OracleCorpusManifest> {
    let metadata = fs::metadata(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(invalid_value("manifest", "manifest exceeds byte limit"));
    }
    let contents = fs::read_to_string(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    config::Config::builder()
        .add_source(config::File::from_str(&contents, config::FileFormat::Yaml))
        .build()
        .map_err(|source| {
            invalid_value(
                "manifest",
                format!("could not parse manifest YAML: {source}"),
            )
        })?
        .try_deserialize()
        .map_err(|source| {
            invalid_value(
                "manifest",
                format!("could not deserialize manifest schema: {source}"),
            )
        })
}

fn corpus_root(manifest_dir: &Path, manifest: &OracleCorpusManifest) -> Result<PathBuf> {
    let configured = env::var_os(CORPUS_ROOT_ENV).map_or_else(
        || manifest_dir.join(manifest.corpus_root.as_str()),
        PathBuf::from,
    );
    let canonical = configured.canonicalize().map_err(|source| PdfvError::Io {
        path: Some(configured),
        source,
    })?;
    Ok(canonical)
}

fn run_row(
    row: &pdfv_core::OracleCorpusRow,
    corpus_root: &Path,
    vera_pdf_bin: &str,
    timeout: Duration,
    max_output_bytes: u64,
) -> Result<OracleCorpusResultRow> {
    let path = resolve_row_path(corpus_root, row.path.as_str())?;
    let pdfv = run_pdfv(row, &path);
    let vera_pdf = run_verapdf(row, &path, vera_pdf_bin, timeout, max_output_bytes)?;
    let observation = OracleRowObservation {
        vera_pdf_outcome: vera_pdf.outcome,
        pdfv_outcome: pdfv.outcome,
        pdfv_unsupported_rules: pdfv.unsupported_rules,
        vera_pdf_status: vera_pdf.status,
        pdfv_status: pdfv.status,
    };
    OracleCorpusResultRow::from_observation(row, observation)
}

fn run_pdfv(row: &pdfv_core::OracleCorpusRow, path: &Path) -> ValidatorOutcome {
    let options = row.profile_policy.validation_options();
    match Validator::new(options).and_then(|validator| validator.validate_path(path)) {
        Ok(report) => ValidatorOutcome {
            outcome: Some(CorpusOutcome::from(report.status)),
            status: OracleExecutionStatus::Completed,
            unsupported_rules: report
                .profile_reports
                .iter()
                .map(|profile| u64::try_from(profile.unsupported_rules.len()).unwrap_or(u64::MAX))
                .sum(),
        },
        Err(_) => ValidatorOutcome {
            outcome: None,
            status: OracleExecutionStatus::Failed,
            unsupported_rules: 0,
        },
    }
}

fn run_verapdf(
    row: &pdfv_core::OracleCorpusRow,
    path: &Path,
    vera_pdf_bin: &str,
    timeout: Duration,
    max_output_bytes: u64,
) -> Result<ValidatorOutcome> {
    let mut command = Command::new(vera_pdf_bin);
    command.arg("--format").arg("json");
    if let Some(flavour) = row.profile_policy.verapdf_flavour_code() {
        command.arg("--flavour").arg(flavour);
    }
    command.arg(path);
    let output = run_capture(command, timeout, max_output_bytes)?;
    if output.timed_out {
        return Ok(ValidatorOutcome {
            outcome: None,
            status: OracleExecutionStatus::Timeout,
            unsupported_rules: 0,
        });
    }
    let outcome = verapdf_outcome(&output.stdout, output.exit_code);
    Ok(ValidatorOutcome {
        outcome,
        status: outcome.map_or(OracleExecutionStatus::Failed, |_| {
            OracleExecutionStatus::Completed
        }),
        unsupported_rules: 0,
    })
}

fn run_capture(
    mut command: Command,
    timeout: Duration,
    max_output_bytes: u64,
) -> Result<ProcessOutput> {
    let temp_dir = tempfile::tempdir().map_err(|source| PdfvError::Io { path: None, source })?;
    let stdout_path = temp_dir.path().join("stdout");
    let stderr_path = temp_dir.path().join("stderr");
    let stdout = fs::File::create(&stdout_path).map_err(|source| PdfvError::Io {
        path: Some(stdout_path.clone()),
        source,
    })?;
    let stderr = fs::File::create(&stderr_path).map_err(|source| PdfvError::Io {
        path: Some(stderr_path.clone()),
        source,
    })?;
    let mut child = command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|source| PdfvError::Io { path: None, source })?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| PdfvError::Io { path: None, source })?
        {
            break Some(status);
        }
        if started.elapsed() >= timeout {
            child
                .kill()
                .map_err(|source| PdfvError::Io { path: None, source })?;
            child
                .wait()
                .map_err(|source| PdfvError::Io { path: None, source })?;
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = read_bounded(&stdout_path, max_output_bytes)?;
    let stderr = read_bounded(&stderr_path, max_output_bytes)?;
    Ok(ProcessOutput {
        stdout,
        stderr,
        exit_code: status.and_then(|status| status.code()),
        timed_out: status.is_none(),
    })
}

fn read_bounded(path: &Path, max_output_bytes: u64) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| PdfvError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
    let limit = max_output_bytes.saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| PdfvError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_output_bytes {
        return Err(invalid_value(
            "processOutput",
            "process output exceeds byte limit",
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn verapdf_outcome(stdout: &str, exit_code: Option<i32>) -> Option<CorpusOutcome> {
    let json = serde_json::from_str::<serde_json::Value>(stdout).ok();
    if let Some(value) = &json {
        if numeric_field_gt_zero(value, "failedToParse") {
            return Some(CorpusOutcome::ParseFailed);
        }
        if numeric_field_gt_zero(value, "encrypted") {
            return Some(CorpusOutcome::Encrypted);
        }
        if let Some(is_compliant) = first_bool_field(value, "isCompliant") {
            return Some(if is_compliant {
                CorpusOutcome::Valid
            } else {
                CorpusOutcome::Invalid
            });
        }
    }
    match exit_code {
        Some(0) => Some(CorpusOutcome::Valid),
        Some(1) => Some(CorpusOutcome::Invalid),
        Some(7) => Some(CorpusOutcome::ParseFailed),
        _ => None,
    }
}

fn first_bool_field(value: &serde_json::Value, key: &str) -> Option<bool> {
    match value {
        serde_json::Value::Object(map) => map
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .or_else(|| map.values().find_map(|child| first_bool_field(child, key))),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|child| first_bool_field(child, key))
        }
        _ => None,
    }
}

fn numeric_field_gt_zero(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.get(key)
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|count| count > 0)
                || map.values().any(|child| numeric_field_gt_zero(child, key))
        }
        serde_json::Value::Array(values) => {
            values.iter().any(|child| numeric_field_gt_zero(child, key))
        }
        _ => false,
    }
}

fn resolve_row_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let joined = root.join(relative);
    let canonical = joined.canonicalize().map_err(|source| PdfvError::Io {
        path: Some(joined),
        source,
    })?;
    if !canonical.starts_with(root) {
        return Err(invalid_value("path", "canonical path escapes corpus root"));
    }
    Ok(canonical)
}

fn report_metadata(
    vera_pdf_bin: &str,
    timeout: Duration,
    max_output_bytes: u64,
) -> Result<OracleReportMetadata> {
    Ok(OracleReportMetadata::new(
        BoundedText::new(ENGINE_VERSION, 64)?,
        BoundedText::new(git_output(["rev-parse", "HEAD"]), 128)?,
        BoundedText::new(
            command_banner(vera_pdf_bin, ["--version"], timeout, max_output_bytes),
            512,
        )?,
        BoundedText::new(
            command_banner("java", ["-version"], timeout, max_output_bytes),
            512,
        )?,
        BoundedText::new(env::consts::OS, 64)?,
        BoundedText::new(env::consts::ARCH, 64)?,
        vendor_pins(),
    ))
}

fn command_banner<const N: usize>(
    program: &str,
    args: [&str; N],
    timeout: Duration,
    max_output_bytes: u64,
) -> String {
    let mut command = Command::new(program);
    command.args(args);
    run_capture(command, timeout, max_output_bytes).map_or_else(
        |error| format!("unavailable: {error}"),
        |output| {
            let joined = if output.stdout.trim().is_empty() {
                output.stderr
            } else {
                output.stdout
            };
            joined.lines().next().unwrap_or("unknown").to_owned()
        },
    )
}

fn git_output<const N: usize>(args: [&str; N]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("unknown"))
}

fn vendor_pins() -> BTreeMap<String, String> {
    [
        ("veraPDF-apps", "vendors/veraPDF-apps"),
        ("veraPDF-validation", "vendors/veraPDF-validation"),
        ("veraPDF-parser", "vendors/veraPDF-parser"),
        ("veraPDF-library", "vendors/veraPDF-library"),
    ]
    .into_iter()
    .map(|(name, path)| {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| String::from("unknown"));
        (String::from(name), output)
    })
    .collect()
}

fn write_artifacts(report: &OracleCorpusReport) -> Result<()> {
    let output_dir = Path::new(OUTPUT_DIR);
    fs::create_dir_all(output_dir).map_err(|source| PdfvError::Io {
        path: Some(output_dir.to_path_buf()),
        source,
    })?;
    write_json_file(&output_dir.join("oracle-corpus.json"), report)?;
    write_json_file(&output_dir.join("oracle-summary.json"), &report.summary)?;
    write_json_file(
        &output_dir.join("oracle-drift.json"),
        &oracle_drift_report(report)?,
    )
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

fn required_path_env(name: &'static str) -> Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| invalid_value(name, format!("{name} must be set")))
}

fn env_u64(name: &'static str, default: u64) -> Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|source| invalid_value(name, format!("invalid integer value: {source}"))),
        Err(env::VarError::NotPresent) => Ok(default),
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

#[derive(Debug)]
struct ValidatorOutcome {
    outcome: Option<CorpusOutcome>,
    status: OracleExecutionStatus,
    unsupported_rules: u64,
}

#[derive(Debug)]
struct ProcessOutput {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    timed_out: bool,
}

#[cfg(test)]
mod tests {
    use pdfv_core::CorpusOutcome;
    use serde_json::json;

    use super::{first_bool_field, numeric_field_gt_zero, verapdf_outcome};

    #[test]
    fn test_should_parse_recursive_verapdf_json_compliance() {
        let value = json!({"jobs": [{"validationReport": {"isCompliant": false}}]});

        assert_eq!(first_bool_field(&value, "isCompliant"), Some(false));
    }

    #[test]
    fn test_should_prefer_failed_to_parse_batch_summary() {
        let value = r#"{"batchSummary":{"failedToParse":1},"jobs":[]}"#;

        assert_eq!(
            verapdf_outcome(value, Some(1)),
            Some(CorpusOutcome::ParseFailed)
        );
    }

    #[test]
    fn test_should_find_numeric_fields_recursively() {
        let value = json!({"outer": [{"batchSummary": {"encrypted": 1}}]});

        assert!(numeric_field_gt_zero(&value, "encrypted"));
    }
}
