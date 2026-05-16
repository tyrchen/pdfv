#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the CLI is a synchronous adapter per spec; Tokio is reserved for future async \
              service integration"
)]
//! Command-line entrypoint for pdfv.

use std::{
    fs::File,
    io::{self, Read, Write},
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pdfv_core::{
    BatchReport, BoundedText, BuiltinProfileRepository, FlavourSelection, MaxDisplayedFailures,
    PasswordSecret, PdfvError, ReportFormat, ResourceLimits, ValidationFlavour, ValidationOptions,
    ValidationStatus, ValidationWarning, Validator,
};
use rayon::prelude::*;
use serde::Deserialize;

const EXIT_VALID: u8 = 0;
const EXIT_INVALID: u8 = 1;
const EXIT_PARSE_FAILED: u8 = 2;
const EXIT_ENCRYPTED: u8 = 3;
const EXIT_INCOMPLETE: u8 = 4;
const EXIT_USAGE: u8 = 64;
const EXIT_INTERNAL: u8 = 70;
const MAX_CLI_JOBS: u32 = 256;
const HARD_MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const HARD_MAX_OBJECTS: u64 = 8_388_607;
const HARD_MAX_OBJECT_DEPTH: u32 = 512;
const HARD_MAX_ARRAY_LEN: u64 = 1_000_000;
const HARD_MAX_DICT_ENTRIES: u64 = 100_000;
const HARD_MAX_NAME_BYTES: usize = 4096;
const HARD_MAX_STRING_BYTES: usize = 16 * 1024 * 1024;
const HARD_MAX_PASSWORD_BYTES: usize = 4096;
const HARD_MAX_STREAM_BYTES: u64 = 1024 * 1024 * 1024;
const HARD_MAX_PARSE_FACTS: usize = 1_000_000;
const HARD_MAX_ENCRYPTION_DICT_ENTRIES: u64 = 1024;

/// Command-line arguments for the pdfv binary.
#[derive(Debug, Parser)]
#[command(name = "pdfv", version, about = "Validate PDF conformance")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// pdfv subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Validate one or more PDF files.
    Validate(Box<ValidateArgs>),
    /// Inspect built-in validation profiles.
    Profiles {
        /// Profile catalog command.
        #[command(subcommand)]
        command: ProfilesCommand,
    },
}

/// `pdfv profiles` subcommands.
#[derive(Debug, Subcommand)]
enum ProfilesCommand {
    /// List available built-in validation profiles.
    List,
}

/// Arguments for `pdfv validate`.
#[derive(Debug, Args)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI flags are independent clap switches; grouping them would make the command \
              surface less direct"
)]
struct ValidateArgs {
    /// PDF files to validate.
    #[arg(value_name = "PATH", required = true)]
    paths: Vec<PathBuf>,
    /// Output format.
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    /// Built-in validation flavour or `auto`.
    #[arg(long, value_parser = parse_flavour_selection)]
    flavour: Option<FlavourSelection>,
    /// Custom profile path. Custom profile loading is not available in M0.
    #[arg(long, value_name = "PATH", conflicts_with = "flavour")]
    profile: Option<PathBuf>,
    /// Maximum failed assertion details retained per rule. Use -1 for no practical cap.
    #[arg(long, allow_hyphen_values = true, value_parser = parse_max_failures)]
    max_failures: Option<MaxDisplayedFailures>,
    /// Recursively discover PDF files under directories.
    #[arg(long)]
    recursive: bool,
    /// Maximum concurrent validation jobs.
    #[arg(long, default_value = "1", value_parser = parse_jobs)]
    jobs: NonZeroU32,
    /// YAML config file. CLI flags override values from this file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Write the report to a file instead of stdout.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Omit input paths from reports.
    #[arg(long)]
    redact_paths: bool,
    /// Record passed assertions.
    #[arg(long)]
    record_passes: bool,
    /// Read the PDF password from stdin.
    #[arg(long, conflicts_with_all = ["password_file", "password_env"])]
    password_stdin: bool,
    /// Read the PDF password from a file.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["password_stdin", "password_env"])]
    password_file: Option<PathBuf>,
    /// Read the PDF password from an environment variable name.
    #[arg(long, value_name = "ENV_VAR", conflicts_with_all = ["password_stdin", "password_file"])]
    password_env: Option<String>,
}

/// CLI output format values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum FormatArg {
    /// Compact JSON output.
    Json,
    /// Pretty JSON output.
    JsonPretty,
    /// Human-readable text output.
    Text,
    /// Machine-readable XML compatibility output.
    Xml,
    /// Deprecated compatibility alias for XML output.
    Mrr,
}

impl From<FormatArg> for ReportFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Json => Self::Json,
            FormatArg::JsonPretty => Self::JsonPretty,
            FormatArg::Text => Self::Text,
            FormatArg::Xml | FormatArg::Mrr => Self::Xml,
        }
    }
}

impl FormatArg {
    fn into_report_format(self) -> ReportFormat {
        ReportFormat::from(self)
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit = if matches!(error.kind(), clap::error::ErrorKind::DisplayVersion) {
                EXIT_VALID
            } else {
                EXIT_USAGE
            };
            if let Err(write_error) = error.print() {
                let _ = writeln!(io::stderr(), "failed to write CLI error: {write_error}");
                return ExitCode::from(EXIT_INTERNAL);
            }
            return ExitCode::from(exit);
        }
    };

    match run(cli) {
        Ok(exit) => ExitCode::from(exit.code()),
        Err(error) => {
            let exit = exit_for_error(error.downcast_ref::<PdfvError>());
            let _ = writeln!(io::stderr(), "{error:#}");
            ExitCode::from(exit)
        }
    }
}

fn run(cli: Cli) -> Result<CliExit> {
    match cli.command {
        Command::Validate(args) => run_validate(&args),
        Command::Profiles {
            command: ProfilesCommand::List,
        } => run_profiles_list(),
    }
}

fn run_validate(args: &ValidateArgs) -> Result<CliExit> {
    let started = Instant::now();
    let config = args
        .config
        .as_ref()
        .map(|path| load_cli_config(path))
        .transpose()?
        .unwrap_or_default();
    let format = args
        .format
        .map_or(config.output.format, FormatArg::into_report_format);
    let options = validation_options(args, &config)?;
    let validator = Validator::new(options).context("failed to initialize validator")?;
    let paths = discover_inputs(&args.paths, args.recursive)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(usize::try_from(args.jobs.get()).unwrap_or(usize::MAX))
        .build()
        .context("failed to build validation worker pool")?;
    let mut batch = pool.install(|| validate_paths(&validator, &paths));
    let reports = &mut batch.reports;
    if args.redact_paths || config.output.redact_paths {
        redact_report_paths(reports);
    }
    let exit = reports
        .iter()
        .map(|report| CliExit::from_status(report.status))
        .fold(CliExit::Valid, CliExit::worst);
    let exit = if batch.internal_errors > 0 {
        CliExit::worst(exit, CliExit::Internal)
    } else {
        exit
    };

    if let Some(output_path) = args.output.as_ref().or(config.output.path.as_ref()) {
        let mut output = File::create(output_path)
            .with_context(|| format!("failed to create {}", output_path.display()))?;
        write_reports(format, batch, started, args.recursive, &mut output)?;
        output.flush().context("failed to flush report output")?;
    } else {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        write_reports(format, batch, started, args.recursive, &mut handle)?;
        handle.flush().context("failed to flush report output")?;
    }
    Ok(exit)
}

fn run_profiles_list() -> Result<CliExit> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for entry in BuiltinProfileRepository::new()
        .list_profiles()
        .context("failed to list profiles")?
    {
        writeln!(
            handle,
            "{}\tpdfa-{}{}\t{}",
            entry.identity.id.as_str(),
            entry.flavour.part,
            entry.flavour.conformance.as_str(),
            entry.identity.name.as_str(),
        )
        .context("failed to write profile list")?;
    }
    handle.flush().context("failed to flush profile list")?;
    Ok(CliExit::Valid)
}

fn write_reports<W: Write>(
    format: ReportFormat,
    batch: ValidationBatch,
    started: Instant,
    force_batch: bool,
    output: &mut W,
) -> Result<()> {
    if batch.reports.len() == 1 && batch.internal_errors == 0 && !force_batch {
        let reports = batch.reports;
        let Some(report) = reports.first() else {
            return Err(anyhow::anyhow!("validation produced no reports"));
        };
        format
            .write_report(report, output)
            .context("failed to write validation report")?;
    } else {
        let report = BatchReport::from_items_with_internal_errors(
            batch.reports,
            batch.warnings,
            started.elapsed(),
            batch.internal_errors,
        );
        format
            .write_batch(&report, output)
            .context("failed to write batch report")?;
    }
    Ok(())
}

fn validation_options(args: &ValidateArgs, config: &CliConfig) -> Result<ValidationOptions> {
    let flavour = args.profile.as_ref().map_or_else(
        || config.validation.flavour_selection(args.flavour.clone()),
        |profile_path| {
            Ok(FlavourSelection::CustomProfile {
                profile_path: profile_path.clone(),
            })
        },
    )?;
    let resource_limits = validated_resource_limits(config.resources.clone().unwrap_or_default())?;
    let password = resolve_password(args, config, resource_limits.max_password_bytes)?;
    Ok(ValidationOptions::builder()
        .flavour(flavour)
        .resource_limits(resource_limits)
        .password(password)
        .max_failed_assertions_per_rule(
            args.max_failures
                .or(config.validation.max_failed_assertions_per_rule)
                .unwrap_or_default(),
        )
        .record_passed_assertions(args.record_passes || config.validation.record_passed_assertions)
        .build())
}

fn resolve_password(
    args: &ValidateArgs,
    config: &CliConfig,
    max_password_bytes: usize,
) -> Result<Option<PasswordSecret>> {
    let cli_source = PasswordSource::from_args(args)?;
    let config_source = config.validation.password_source()?;
    let Some(source) = cli_source.or(config_source) else {
        return Ok(None);
    };
    let value = match source {
        PasswordSource::Stdin => {
            let mut bytes = Vec::new();
            let read_limit = u64::try_from(max_password_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            io::stdin()
                .take(read_limit)
                .read_to_end(&mut bytes)
                .context("failed to read password from stdin")?;
            if bytes.len() > max_password_bytes {
                return Err(password_config_error(
                    "passwordStdin",
                    "password stdin exceeds byte limit",
                )
                .into());
            }
            let value = String::from_utf8(bytes)
                .map_err(|_| password_config_error("passwordStdin", "password is not UTF-8"))?;
            trim_one_line_ending(value)
        }
        PasswordSource::File(path) => {
            let metadata = std::fs::metadata(&path)
                .with_context(|| format!("failed to inspect password file {}", path.display()))?;
            if metadata.is_dir() {
                return Err(
                    password_config_error("passwordFile", "password file is a directory").into(),
                );
            }
            let max = u64::try_from(max_password_bytes).unwrap_or(u64::MAX);
            if metadata.len() > max {
                return Err(password_config_error(
                    "passwordFile",
                    "password file exceeds byte limit",
                )
                .into());
            }
            let value = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read password file {}", path.display()))?;
            trim_one_line_ending(value)
        }
        PasswordSource::Env(name) => std::env::var(&name)
            .with_context(|| format!("failed to read password environment variable {name}"))?,
    };
    PasswordSecret::new_with_limit(value, max_password_bytes)
        .map(Some)
        .map_err(anyhow::Error::from)
}

fn trim_one_line_ending(mut value: String) -> String {
    if value.ends_with("\r\n") {
        value.truncate(value.len().saturating_sub(2));
    } else if value.ends_with('\n') || value.ends_with('\r') {
        value.truncate(value.len().saturating_sub(1));
    }
    value
}

fn parse_max_failures(value: &str) -> std::result::Result<MaxDisplayedFailures, String> {
    let value = value
        .parse::<i64>()
        .map_err(|_| String::from("max failures must be -1 or a positive integer"))?;
    match value {
        -1 => Ok(MaxDisplayedFailures::new(NonZeroU32::MAX)),
        1.. if value <= i64::from(u32::MAX) => {
            let value =
                u32::try_from(value).map_err(|_| String::from("max failures exceeds u32 range"))?;
            let non_zero = NonZeroU32::new(value)
                .ok_or_else(|| String::from("max failures must be greater than zero"))?;
            Ok(MaxDisplayedFailures::new(non_zero))
        }
        _ => Err(format!(
            "max failures must be -1 or an integer in 1..={}",
            u32::MAX
        )),
    }
}

fn parse_jobs(value: &str) -> std::result::Result<NonZeroU32, String> {
    let jobs = value
        .parse::<u32>()
        .map_err(|_| String::from("jobs must be a positive integer"))?;
    let jobs =
        NonZeroU32::new(jobs).ok_or_else(|| String::from("jobs must be greater than zero"))?;
    if jobs.get() > MAX_CLI_JOBS {
        return Err(format!("jobs must be in 1..={MAX_CLI_JOBS}"));
    }
    Ok(jobs)
}

fn parse_flavour_selection(value: &str) -> std::result::Result<FlavourSelection, String> {
    if value == "auto" {
        return Ok(FlavourSelection::Auto { default: None });
    }
    parse_flavour(value).map(|flavour| FlavourSelection::Explicit { flavour })
}

fn parse_flavour(value: &str) -> std::result::Result<ValidationFlavour, String> {
    let Some(rest) = value.strip_prefix("pdfa-") else {
        return Err(String::from(
            "expected auto or a PDF/A flavour such as pdfa-1b",
        ));
    };
    if rest.len() < 2 {
        return Err(String::from("expected PDF/A flavour part and conformance"));
    }
    let split_at = rest
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| String::from("expected conformance level after PDF/A part"))?;
    let (part, conformance) = rest.split_at(split_at);
    if part.is_empty() || conformance.is_empty() {
        return Err(String::from("expected PDF/A flavour part and conformance"));
    }
    let part = part
        .parse::<u32>()
        .map_err(|_| String::from("PDF/A part must be an integer"))?;
    let part = NonZeroU32::new(part).ok_or_else(|| String::from("PDF/A part must be non-zero"))?;
    ValidationFlavour::new("pdfa", part, conformance)
        .map_err(|error| format!("invalid PDF/A flavour: {error}"))
}

fn load_cli_config(path: &Path) -> Result<CliConfig> {
    config::Config::builder()
        .add_source(config::File::from(path).required(true))
        .build()
        .with_context(|| format!("failed to read config {}", path.display()))?
        .try_deserialize()
        .with_context(|| format!("failed to parse config {}", path.display()))
}

fn discover_inputs(paths: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut discovered = Vec::new();
    for path in paths {
        if recursive && path.is_dir() {
            discover_directory(path, &mut discovered)?;
        } else {
            discovered.push(path.clone());
        }
    }
    if discovered.is_empty() {
        return Err(
            PdfvError::Configuration(pdfv_core::ConfigError::InvalidValue {
                field: "paths",
                reason: pdfv_core::BoundedText::new("no PDF inputs discovered", 256)?,
            })
            .into(),
        );
    }
    Ok(discovered)
}

fn discover_directory(root: &Path, discovered: &mut Vec<PathBuf>) -> Result<()> {
    const MAX_DISCOVERED_FILES: usize = 100_000;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path)
            .with_context(|| format!("failed to read directory {}", path.display()))?
        {
            let entry = entry
                .with_context(|| format!("failed to read directory entry in {}", path.display()))?;
            let entry_path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", entry_path.display()))?;
            if file_type.is_dir() {
                stack.push(entry_path);
            } else if file_type.is_file() && is_pdf_path(&entry_path) {
                if discovered.len() >= MAX_DISCOVERED_FILES {
                    return Err(
                        PdfvError::Configuration(pdfv_core::ConfigError::InvalidValue {
                            field: "paths",
                            reason: pdfv_core::BoundedText::new(
                                "recursive discovery exceeded file limit",
                                256,
                            )?,
                        })
                        .into(),
                    );
                }
                discovered.push(entry_path);
            }
        }
    }
    discovered.sort();
    Ok(())
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn validate_paths(validator: &Validator, paths: &[PathBuf]) -> ValidationBatch {
    let results = paths
        .par_iter()
        .map(|path| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                validator
                    .validate_path(path)
                    .with_context(|| format!("failed to validate {}", path.display()))
            }));
            match result {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err(format!(
                    "validation worker panicked while processing {}",
                    path.display()
                )),
            }
        })
        .collect::<Vec<_>>();
    let mut batch = ValidationBatch::default();
    for result in results {
        match result {
            Ok(report) => batch.reports.push(report),
            Err(message) => {
                batch.internal_errors = batch.internal_errors.saturating_add(1);
                batch.warnings.push(ValidationWarning::General {
                    message: BoundedText::new(message, 512).unwrap_or_else(|_| {
                        BoundedText::new("internal validation error", 128)
                            .unwrap_or_else(|_| unreachable_bounded_text())
                    }),
                });
            }
        }
    }
    batch
}

fn redact_report_paths(reports: &mut [pdfv_core::ValidationReport]) {
    for report in reports {
        report.source.path = None;
    }
}

fn validated_resource_limits(limits: ResourceLimits) -> Result<ResourceLimits> {
    ensure_limit("maxFileBytes", limits.max_file_bytes, HARD_MAX_FILE_BYTES)?;
    ensure_limit("maxObjects", limits.max_objects, HARD_MAX_OBJECTS)?;
    ensure_limit_u32(
        "maxObjectDepth",
        limits.max_object_depth,
        HARD_MAX_OBJECT_DEPTH,
    )?;
    ensure_limit("maxArrayLen", limits.max_array_len, HARD_MAX_ARRAY_LEN)?;
    ensure_limit(
        "maxDictEntries",
        limits.max_dict_entries,
        HARD_MAX_DICT_ENTRIES,
    )?;
    ensure_limit_usize("maxNameBytes", limits.max_name_bytes, HARD_MAX_NAME_BYTES)?;
    ensure_limit_usize(
        "maxStringBytes",
        limits.max_string_bytes,
        HARD_MAX_STRING_BYTES,
    )?;
    ensure_limit_usize(
        "maxPasswordBytes",
        limits.max_password_bytes,
        HARD_MAX_PASSWORD_BYTES,
    )?;
    ensure_limit_usize(
        "maxDecryptedStringBytes",
        limits.max_decrypted_string_bytes,
        HARD_MAX_STRING_BYTES,
    )?;
    ensure_limit(
        "maxStreamDeclaredBytes",
        limits.max_stream_declared_bytes,
        HARD_MAX_STREAM_BYTES,
    )?;
    ensure_limit(
        "maxStreamDecodeBytes",
        limits.max_stream_decode_bytes,
        HARD_MAX_STREAM_BYTES,
    )?;
    ensure_limit(
        "maxDecryptedStreamBytes",
        limits.max_decrypted_stream_bytes,
        HARD_MAX_STREAM_BYTES,
    )?;
    ensure_limit(
        "maxEncryptionDictEntries",
        limits.max_encryption_dict_entries,
        HARD_MAX_ENCRYPTION_DICT_ENTRIES,
    )?;
    ensure_limit_usize(
        "maxParseFacts",
        limits.max_parse_facts,
        HARD_MAX_PARSE_FACTS,
    )?;
    Ok(limits)
}

fn password_config_error(field: &'static str, reason: &'static str) -> PdfvError {
    PdfvError::Configuration(pdfv_core::ConfigError::InvalidValue {
        field,
        reason: BoundedText::new(reason, 128).unwrap_or_else(|_| unreachable_bounded_text()),
    })
}

fn ensure_limit(field: &'static str, value: u64, max: u64) -> Result<()> {
    if value <= max {
        Ok(())
    } else {
        Err(config_limit_error(field, max).into())
    }
}

fn ensure_limit_u32(field: &'static str, value: u32, max: u32) -> Result<()> {
    ensure_limit(field, u64::from(value), u64::from(max))
}

fn ensure_limit_usize(field: &'static str, value: usize, max: usize) -> Result<()> {
    let value = u64::try_from(value).unwrap_or(u64::MAX);
    let max = u64::try_from(max).unwrap_or(u64::MAX);
    ensure_limit(field, value, max)
}

fn config_limit_error(field: &'static str, max: u64) -> PdfvError {
    PdfvError::Configuration(pdfv_core::ConfigError::InvalidValue {
        field,
        reason: BoundedText::new(format!("value exceeds hard cap {max}"), 128)
            .unwrap_or_else(|_| unreachable_bounded_text()),
    })
}

fn unreachable_bounded_text() -> BoundedText {
    BoundedText::new("bounded diagnostic unavailable", 128)
        .unwrap_or_else(|_| std::process::abort())
}

#[derive(Debug, Default)]
struct ValidationBatch {
    reports: Vec<pdfv_core::ValidationReport>,
    warnings: Vec<ValidationWarning>,
    internal_errors: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CliConfig {
    #[serde(default)]
    validation: ValidationConfig,
    #[serde(default)]
    resources: Option<pdfv_core::ResourceLimits>,
    #[serde(default)]
    output: OutputConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ValidationConfig {
    #[serde(default)]
    flavour: Option<String>,
    #[serde(default)]
    max_failed_assertions_per_rule: Option<MaxDisplayedFailures>,
    #[serde(default)]
    record_passed_assertions: bool,
    #[serde(default)]
    password: Option<PasswordConfig>,
}

impl ValidationConfig {
    fn flavour_selection(&self, cli_flavour: Option<FlavourSelection>) -> Result<FlavourSelection> {
        if let Some(flavour) = cli_flavour {
            return Ok(flavour);
        }
        self.flavour
            .as_deref()
            .map(parse_flavour_selection)
            .transpose()
            .map_err(|message| anyhow::anyhow!("invalid config validation.flavour: {message}"))?
            .map_or_else(|| Ok(FlavourSelection::default()), Ok)
    }

    fn password_source(&self) -> Result<Option<PasswordSource>> {
        self.password
            .as_ref()
            .map(PasswordSource::try_from_config)
            .transpose()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasswordConfig {
    #[serde(default)]
    stdin: bool,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    env: Option<String>,
}

#[derive(Clone, Debug)]
enum PasswordSource {
    Stdin,
    File(PathBuf),
    Env(String),
}

impl PasswordSource {
    fn from_args(args: &ValidateArgs) -> Result<Option<Self>> {
        let mut sources = Vec::new();
        if args.password_stdin {
            sources.push(Self::Stdin);
        }
        if let Some(path) = &args.password_file {
            sources.push(Self::File(path.clone()));
        }
        if let Some(name) = &args.password_env {
            sources.push(Self::Env(validate_env_name(name)?));
        }
        one_password_source(sources)
    }

    fn try_from_config(config: &PasswordConfig) -> Result<Self> {
        let mut sources = Vec::new();
        if config.stdin {
            sources.push(Self::Stdin);
        }
        if let Some(path) = &config.file {
            sources.push(Self::File(path.clone()));
        }
        if let Some(name) = &config.env {
            sources.push(Self::Env(validate_env_name(name)?));
        }
        one_password_source(sources)?.ok_or_else(|| {
            password_config_error("validation.password", "password source is empty").into()
        })
    }
}

fn one_password_source(mut sources: Vec<PasswordSource>) -> Result<Option<PasswordSource>> {
    match sources.len() {
        0 => Ok(None),
        1 => Ok(sources.pop()),
        _ => Err(password_config_error(
            "password",
            "exactly zero or one password source is allowed",
        )
        .into()),
    }
}

fn validate_env_name(name: &str) -> Result<String> {
    let is_valid = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if is_valid {
        Ok(name.to_owned())
    } else {
        Err(password_config_error("passwordEnv", "environment variable name is invalid").into())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputConfig {
    #[serde(default = "default_report_format")]
    format: ReportFormat,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    redact_paths: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: default_report_format(),
            path: None,
            redact_paths: false,
        }
    }
}

fn default_report_format() -> ReportFormat {
    ReportFormat::Json
}

fn exit_for_error(error: Option<&PdfvError>) -> u8 {
    match error {
        Some(PdfvError::Profile(pdfv_core::ProfileError::UnsupportedSelection)) => EXIT_INCOMPLETE,
        Some(PdfvError::Configuration(_)) => EXIT_USAGE,
        _ => EXIT_INTERNAL,
    }
}

/// Ordered CLI exit category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliExit {
    /// All validations passed.
    Valid,
    /// At least one validation failed.
    Invalid,
    /// At least one required rule was unsupported.
    Incomplete,
    /// At least one encrypted input could not be validated.
    Encrypted,
    /// At least one input could not be parsed.
    ParseFailed,
    /// At least one internal processing error occurred.
    Internal,
}

impl CliExit {
    fn from_status(status: ValidationStatus) -> Self {
        match status {
            ValidationStatus::Valid => Self::Valid,
            ValidationStatus::Invalid => Self::Invalid,
            ValidationStatus::Encrypted => Self::Encrypted,
            ValidationStatus::ParseFailed => Self::ParseFailed,
            _ => Self::Incomplete,
        }
    }

    fn worst(left: Self, right: Self) -> Self {
        if left.rank() >= right.rank() {
            left
        } else {
            right
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Valid => EXIT_VALID,
            Self::Invalid => EXIT_INVALID,
            Self::ParseFailed => EXIT_PARSE_FAILED,
            Self::Encrypted => EXIT_ENCRYPTED,
            Self::Incomplete => EXIT_INCOMPLETE,
            Self::Internal => EXIT_INTERNAL,
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Valid => EXIT_VALID,
            Self::Invalid => EXIT_INVALID,
            Self::ParseFailed => EXIT_PARSE_FAILED,
            Self::Encrypted => EXIT_ENCRYPTED,
            Self::Incomplete => EXIT_INCOMPLETE,
            Self::Internal => EXIT_INTERNAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use pdfv_core::{FlavourSelection, ValidationStatus};

    use super::{CliExit, parse_flavour_selection};

    #[test]
    fn test_should_parse_auto_flavour() {
        let result = parse_flavour_selection("auto");

        assert!(matches!(
            result,
            Ok(FlavourSelection::Auto { default: None })
        ));
    }

    #[test]
    fn test_should_parse_pdfa_flavour() {
        let result = parse_flavour_selection("pdfa-1b");

        assert!(matches!(result, Ok(FlavourSelection::Explicit { .. })));
    }

    #[test]
    fn test_should_rank_parse_failed_as_worst_exit() {
        let exit = [ValidationStatus::Invalid, ValidationStatus::ParseFailed]
            .into_iter()
            .map(CliExit::from_status)
            .fold(CliExit::Valid, CliExit::worst);

        assert_eq!(exit.code(), 2);
    }

    #[test]
    fn test_should_rank_higher_exit_code_as_worst_exit() {
        let exit = [ValidationStatus::ParseFailed, ValidationStatus::Incomplete]
            .into_iter()
            .map(CliExit::from_status)
            .fold(CliExit::Valid, CliExit::worst);

        assert_eq!(exit.code(), 4);
    }
}
