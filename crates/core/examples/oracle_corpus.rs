//! Runs the live veraPDF oracle corpus and writes release-readiness artifacts.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use pdfv_core::{
    BoundedText, ConfigError, CorpusOutcome, CorpusTier, ENGINE_VERSION, Identifier,
    OracleCorpusManifest, OracleCorpusReport, OracleCorpusResultRow, OracleExecutionStatus,
    OracleProfilePolicy, OracleReportMetadata, OracleRowObservation, PdfvError, ReportError,
    Result, ValidationFlavour, Validator, oracle_drift_report,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command as AsyncCommand,
    runtime::Builder as RuntimeBuilder,
    time::timeout,
};

#[path = "support/oracle_gates.rs"]
mod oracle_gates;

use oracle_gates::{COMPARABLE_REPORT_SCHEMA, ensure_oracle_gates, oracle_output_dir};

const MANIFEST_ENV: &str = "PDFV_ORACLE_CORPUS_MANIFEST";
const CORPUS_ROOT_ENV: &str = "PDFV_ORACLE_CORPUS_ROOT";
const VERAPDF_BIN_ENV: &str = "PDFV_VERAPDF_BIN";
const TIMEOUT_SECONDS_ENV: &str = "PDFV_ORACLE_TIMEOUT_SECONDS";
const MAX_OUTPUT_BYTES_ENV: &str = "PDFV_ORACLE_MAX_OUTPUT_BYTES";
const DEFAULT_VERAPDF_BIN: &str = "verapdf";
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const MAX_PROCESS_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
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
    let timeout = Duration::from_secs(env_u64(
        TIMEOUT_SECONDS_ENV,
        DEFAULT_TIMEOUT_SECONDS,
        MAX_TIMEOUT_SECONDS,
    )?);
    let max_output_bytes = env_u64(
        MAX_OUTPUT_BYTES_ENV,
        DEFAULT_MAX_OUTPUT_BYTES,
        MAX_PROCESS_OUTPUT_BYTES,
    )?;

    let metadata = report_metadata(&vera_pdf_bin, timeout, max_output_bytes)?;
    let rows = manifest
        .rows
        .iter()
        .map(|row| run_row(row, &corpus_root, &vera_pdf_bin, timeout, max_output_bytes))
        .collect::<Result<Vec<_>>>()?;
    let report = OracleCorpusReport::new(COMPARABLE_REPORT_SCHEMA, true, metadata, rows)?;

    write_artifacts(&report)?;
    ensure_oracle_gates(&report, None)
}

fn read_manifest(path: &Path) -> Result<OracleCorpusManifest> {
    let io_error = |source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    };
    let mut contents = String::new();
    Read::take(
        fs::File::open(path).map_err(io_error)?,
        MAX_MANIFEST_BYTES.saturating_add(1),
    )
    .read_to_string(&mut contents)
    .map_err(io_error)?;
    if u64::try_from(contents.len()).unwrap_or(u64::MAX) > MAX_MANIFEST_BYTES {
        return Err(invalid_value("manifest", "manifest exceeds byte limit"));
    }
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
    let path = resolve_row_path(corpus_root, row.path.as_str(), row.tier)?;
    let (mut pdfv, flavours) = run_pdfv(row, &path);
    let vera_pdf = run_verapdf(row, &path, vera_pdf_bin, timeout, max_output_bytes)?;
    if !vera_pdf.profiles.is_empty()
        && !pdfv.profiles.is_empty()
        && vera_pdf.profiles != pdfv.profiles
    {
        pdfv.status = OracleExecutionStatus::IncompatibleProfiles;
    }
    let observation = OracleRowObservation {
        vera_pdf_outcome: vera_pdf.outcome,
        pdfv_outcome: pdfv.outcome,
        pdfv_unsupported_rules: pdfv.unsupported_rules,
        vera_pdf_status: vera_pdf.status,
        pdfv_status: pdfv.status,
    };
    let mut result = OracleCorpusResultRow::from_observation(row, observation)?;
    if !flavours.is_empty() {
        result.profile_family = resolved_profile_family(&flavours)?;
    }
    Ok(result)
}

fn resolved_profile_family(flavours: &[ValidationFlavour]) -> Result<Identifier> {
    let families = flavours
        .iter()
        .map(|flavour| flavour.family.as_str())
        .collect::<BTreeSet<_>>();
    let family = match (families.len(), families.first()) {
        (1, Some(family)) => family,
        (2, _) if families.contains("pdfa") && families.contains("pdfua") => "pdfa-pdfua",
        _ => "mixed",
    };
    Ok(Identifier::new(family.to_owned())?)
}

fn run_pdfv(
    row: &pdfv_core::OracleCorpusRow,
    path: &Path,
) -> (ValidatorOutcome, Vec<ValidationFlavour>) {
    let options = row.profile_policy.validation_options();
    match Validator::new(options).and_then(|validator| validator.validate_path(path)) {
        Ok(report) => (
            ValidatorOutcome {
                outcome: Some(CorpusOutcome::from(report.status)),
                status: OracleExecutionStatus::Completed,
                profiles: report
                    .profile_reports
                    .iter()
                    .map(|profile| profile.profile.name.as_str().to_owned())
                    .collect(),
                unsupported_rules: report
                    .profile_reports
                    .iter()
                    .map(|profile| {
                        u64::try_from(profile.unsupported_rules.len()).unwrap_or(u64::MAX)
                    })
                    .sum(),
            },
            report.flavours,
        ),
        Err(_) => (
            ValidatorOutcome::failed(OracleExecutionStatus::Failed),
            Vec::new(),
        ),
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
    command.args(["--format", "json", "--maxfailuresdisplayed", "1"]);
    let (option, flavour) = verapdf_profile_arg(&row.profile_policy)?;
    command.arg(option).arg(flavour);
    command.arg(path);
    let Ok(output) = run_capture(command, timeout, max_output_bytes) else {
        return Ok(ValidatorOutcome::failed(OracleExecutionStatus::Failed));
    };
    if output.timed_out {
        return Ok(ValidatorOutcome::failed(OracleExecutionStatus::Timeout));
    }
    Ok(verapdf_outcome(&output.stdout, output.exit_code)
        .unwrap_or_else(|| ValidatorOutcome::failed(OracleExecutionStatus::Failed)))
}

fn verapdf_profile_arg(policy: &OracleProfilePolicy) -> Result<(&'static str, String)> {
    let (option, flavour) = match policy {
        OracleProfilePolicy::AutoDefaultPdfa1b => {
            return Ok(("--defaultflavour", String::from("1b")));
        }
        OracleProfilePolicy::Auto {
            default: Some(flavour),
        } => ("--defaultflavour", flavour),
        OracleProfilePolicy::Explicit { flavour } => ("--flavour", flavour),
        _ => {
            return Err(invalid_value(
                "profilePolicy",
                "use an explicit flavour or configure an automatic fallback for veraPDF",
            ));
        }
    };
    OracleProfilePolicy::Explicit {
        flavour: flavour.clone(),
    }
    .verapdf_flavour_code()
    .map(|code| (option, code))
    .ok_or_else(|| invalid_value("profilePolicy", "profile has no veraPDF CLI equivalent"))
}

fn run_capture(
    command: Command,
    timeout: Duration,
    max_output_bytes: u64,
) -> Result<ProcessOutput> {
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(process_io_error)?;
    runtime.block_on(capture_process(command.into(), timeout, max_output_bytes))
}

async fn capture_process(
    mut command: AsyncCommand,
    deadline: Duration,
    max_output_bytes: u64,
) -> Result<ProcessOutput> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(process_io_error)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid_value("processOutput", "missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid_value("processOutput", "missing stderr pipe"))?;
    let captured = timeout(deadline, async {
        tokio::try_join!(
            read_bounded(stdout, max_output_bytes),
            read_bounded(stderr, max_output_bytes),
            async { child.wait().await.map_err(process_io_error) }
        )
    })
    .await;
    match captured {
        Ok(Ok((stdout, stderr, status))) => Ok(ProcessOutput {
            stdout,
            stderr,
            exit_code: status.code(),
            timed_out: false,
        }),
        Ok(Err(error)) => {
            child.kill().await.map_err(process_io_error)?;
            Err(error)
        }
        Err(_) => {
            child.kill().await.map_err(process_io_error)?;
            Ok(ProcessOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: true,
            })
        }
    }
}

async fn read_bounded(reader: impl AsyncRead + Unpin, max_output_bytes: u64) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(max_output_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(process_io_error)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_output_bytes {
        return Err(invalid_value(
            "processOutput",
            "process output exceeds byte limit",
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn process_io_error(source: io::Error) -> PdfvError {
    PdfvError::Io { path: None, source }
}

fn verapdf_outcome(stdout: &str, exit_code: Option<i32>) -> Option<ValidatorOutcome> {
    let VeraPdfOutput { report } = serde_json::from_str(stdout).ok()?;
    let [job] = report.jobs.as_slice() else {
        return None;
    };
    let summary = report.batch_summary;
    if summary.total_jobs != 1 || summary.out_of_memory != 0 || summary.vera_exceptions != 0 {
        return None;
    }
    let outcomes = job.validation_result.as_slice();
    if outcomes
        .iter()
        .any(|result| !matches!(result.job_end_status, VeraPdfJobEndStatus::Normal))
    {
        return None;
    }
    let outcome = match (
        exit_code,
        summary.failed_parsing_jobs,
        summary.failed_encrypted_jobs,
        outcomes,
    ) {
        (Some(7), 1, 0, []) => CorpusOutcome::ParseFailed,
        (Some(8), 0, 1, []) => CorpusOutcome::Encrypted,
        (Some(0), 0, 0, [_, ..]) if outcomes.iter().all(|result| result.compliant) => {
            CorpusOutcome::Valid
        }
        (Some(1), 0, 0, [_, ..]) if outcomes.iter().any(|result| !result.compliant) => {
            CorpusOutcome::Invalid
        }
        _ => return None,
    };
    let profiles = outcomes
        .iter()
        .map(|result| result.profile_name.as_str().to_owned())
        .collect();
    Some(ValidatorOutcome {
        outcome: Some(outcome),
        status: OracleExecutionStatus::Completed,
        profiles,
        unsupported_rules: 0,
    })
}

#[derive(Debug, Deserialize)]
struct VeraPdfOutput {
    report: VeraPdfReport,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeraPdfReport {
    jobs: Vec<VeraPdfJob>,
    batch_summary: VeraPdfBatchSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeraPdfJob {
    #[serde(default)]
    validation_result: Vec<VeraPdfValidationResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeraPdfValidationResult {
    compliant: bool,
    job_end_status: VeraPdfJobEndStatus,
    profile_name: BoundedText,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum VeraPdfJobEndStatus {
    Normal,
    #[serde(other)]
    Failed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VeraPdfBatchSummary {
    total_jobs: u32,
    out_of_memory: u32,
    vera_exceptions: u32,
    failed_parsing_jobs: u32,
    failed_encrypted_jobs: u32,
}

fn resolve_row_path(root: &Path, relative: &str, tier: CorpusTier) -> Result<PathBuf> {
    let joined = root.join(relative);
    let canonical = joined
        .canonicalize()
        .map_err(|source| PdfvError::Io { path: None, source })?;
    if !canonical.starts_with(root) {
        return Err(invalid_value("path", "canonical path escapes corpus root"));
    }
    let canonical_relative = canonical
        .strip_prefix(root)
        .map_err(|_| invalid_value("path", "canonical path escapes corpus root"))?;
    if tier == CorpusTier::T3RealWorld
        && canonical_relative
            .components()
            .map(|component| component.as_os_str().to_str())
            .ne(relative.split('/').map(Some))
    {
        return Err(invalid_value(
            "path",
            "T3 paths must name canonical files directly",
        ));
    }
    if !fs::metadata(&canonical)
        .map_err(|source| PdfvError::Io { path: None, source })?
        .is_file()
    {
        return Err(invalid_value(
            "path",
            "corpus path must name a regular file",
        ));
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
    let output_dir = oracle_output_dir();
    fs::create_dir_all(&output_dir).map_err(|source| PdfvError::Io {
        path: Some(output_dir.clone()),
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

fn required_path_env(name: &'static str) -> Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| invalid_value(name, format!("{name} must be set")))
}

fn env_u64(name: &'static str, default: u64, max: u64) -> Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|parsed| (1..=max).contains(parsed))
            .ok_or_else(|| {
                invalid_value(name, format!("expected an integer from 1 through {max}"))
            }),
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
    profiles: BTreeSet<String>,
    unsupported_rules: u64,
}

impl ValidatorOutcome {
    fn failed(status: OracleExecutionStatus) -> Self {
        Self {
            outcome: None,
            status,
            profiles: BTreeSet::new(),
            unsupported_rules: 0,
        }
    }
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
    use std::num::NonZeroU32;
    #[cfg(unix)]
    use std::{
        error::Error, fs, os::unix::fs::symlink, process::Command, result::Result as StdResult,
        time::Duration,
    };

    use pdfv_core::{CorpusOutcome, OracleProfilePolicy, Result, ValidationFlavour};
    use rstest::rstest;

    #[cfg(unix)]
    use super::{resolve_row_path, run_capture};
    use super::{verapdf_outcome, verapdf_profile_arg};

    #[rstest]
    #[case(&[true], 0, Some(CorpusOutcome::Valid))]
    #[case(&[false], 1, Some(CorpusOutcome::Invalid))]
    #[case(&[true, false], 1, Some(CorpusOutcome::Invalid))]
    #[case(&[false, true], 1, Some(CorpusOutcome::Invalid))]
    #[case(&[true, false], 0, None)]
    #[case(&[true], 3, None)]
    #[case(&[], 0, None)]
    fn test_should_classify_complete_validation_reports(
        #[case] compliant: &[bool],
        #[case] exit_code: i32,
        #[case] expected: Option<CorpusOutcome>,
    ) {
        let results = compliant
            .iter()
            .enumerate()
            .map(|(index, compliant)| {
                serde_json::json!({
                    "compliant": compliant, "jobEndStatus": "normal", "profileName": format!("profile-{index}"),
                })
            })
            .collect::<Vec<_>>();

        let output = vera_pdf_report(serde_json::json!({"validationResult": results}), 0, 0);

        let result = verapdf_outcome(&output, Some(exit_code));
        if let Some(result) = &result {
            assert_eq!(result.profiles.len(), compliant.len());
        }
        assert_eq!(result.and_then(|result| result.outcome), expected);
    }

    #[rstest]
    #[case(1, 0, 7, Some(CorpusOutcome::ParseFailed))]
    #[case(0, 1, 8, Some(CorpusOutcome::Encrypted))]
    #[case(0, 0, 7, None)]
    #[case(1, 0, 0, None)]
    fn test_should_require_exit_code_and_summary_to_agree(
        #[case] parse_failures: u32,
        #[case] encrypted: u32,
        #[case] exit_code: i32,
        #[case] expected: Option<CorpusOutcome>,
    ) {
        let output = vera_pdf_report(serde_json::json!({}), parse_failures, encrypted);

        assert_eq!(
            verapdf_outcome(&output, Some(exit_code)).and_then(|result| result.outcome),
            expected,
        );
    }

    #[test]
    fn test_should_reject_partial_or_invalid_json_output() {
        let incomplete = vera_pdf_report(
            serde_json::json!({"validationResult": [
                {"compliant": true, "jobEndStatus": "normal", "profileName": "profile-a"},
                {"compliant": false, "jobEndStatus": "error", "profileName": "profile-b"},
            ]}),
            0,
            0,
        );

        assert!(verapdf_outcome(&incomplete, Some(1)).is_none());
        assert!(verapdf_outcome("", Some(0)).is_none());
        assert!(verapdf_outcome("{", Some(1)).is_none());
    }

    #[test]
    fn test_should_forward_explicit_and_fallback_flavours() -> Result<()> {
        let flavour = ValidationFlavour::new(
            "pdfa",
            NonZeroU32::new(4).ok_or(pdfv_core::ProfileError::UnsupportedSelection)?,
            "none",
        )?;
        assert_eq!(
            verapdf_profile_arg(&OracleProfilePolicy::Explicit {
                flavour: flavour.clone(),
            })?,
            ("--flavour", String::from("4")),
        );
        assert_eq!(
            verapdf_profile_arg(&OracleProfilePolicy::Auto {
                default: Some(flavour),
            })?,
            ("--defaultflavour", String::from("4")),
        );
        assert!(verapdf_profile_arg(&OracleProfilePolicy::Auto { default: None }).is_err());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_should_stop_capture_at_the_output_limit() -> Result<()> {
        let mut command = Command::new("printf");
        command.arg("%s").arg("test");
        assert_eq!(
            run_capture(command, Duration::from_secs(2), 4)?.stdout,
            "test"
        );

        let mut command = Command::new("printf");
        command.arg("%s").arg("exceeds limit");
        assert!(matches!(
            run_capture(command, Duration::from_secs(2), 4),
            Err(pdfv_core::PdfvError::Configuration(
                pdfv_core::ConfigError::InvalidValue {
                    field: "processOutput",
                    ..
                }
            )),
        ));
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_should_mark_timed_out_process_capture() -> Result<()> {
        let mut command = Command::new("sleep");
        command.arg("5");
        assert!(run_capture(command, Duration::from_millis(30), 64)?.timed_out);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_should_require_a_direct_regular_t3_document() -> StdResult<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let document = root.join("file.pdf");
        fs::write(&document, b"%PDF-1.4")?;
        symlink(&document, root.join("alias.pdf"))?;
        fs::create_dir(root.join("directory.pdf"))?;

        let tier = pdfv_core::CorpusTier::T3RealWorld;
        assert_eq!(resolve_row_path(&root, "file.pdf", tier)?, document);
        assert!(resolve_row_path(&root, "alias.pdf", tier).is_err());
        assert!(resolve_row_path(&root, "directory.pdf", tier).is_err());
        Ok(())
    }

    fn vera_pdf_report(job: serde_json::Value, parse_failures: u32, encrypted: u32) -> String {
        serde_json::json!({
            "report": {
                "jobs": [job],
                "batchSummary": {
                    "totalJobs": 1,
                    "outOfMemory": 0,
                    "veraExceptions": 0,
                    "failedParsingJobs": parse_failures,
                    "failedEncryptedJobs": encrypted,
                },
            },
        })
        .to_string()
    }
}
