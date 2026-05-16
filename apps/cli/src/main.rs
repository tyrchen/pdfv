#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
//! Command-line entrypoint for pdfv.

use std::{
    io::{self, Write},
    num::NonZeroU32,
    path::PathBuf,
    process::ExitCode,
    time::Instant,
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pdfv_core::{
    BatchReport, FlavourSelection, MaxDisplayedFailures, PdfvError, ReportFormat,
    ValidationFlavour, ValidationOptions, ValidationStatus, Validator,
};

const EXIT_VALID: u8 = 0;
const EXIT_INVALID: u8 = 1;
const EXIT_PARSE_FAILED: u8 = 2;
const EXIT_ENCRYPTED: u8 = 3;
const EXIT_INCOMPLETE: u8 = 4;
const EXIT_USAGE: u8 = 64;
const EXIT_INTERNAL: u8 = 70;

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
    Validate(ValidateArgs),
}

/// Arguments for `pdfv validate`.
#[derive(Debug, Args)]
struct ValidateArgs {
    /// PDF files to validate.
    #[arg(value_name = "PATH", required = true)]
    paths: Vec<PathBuf>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Json)]
    format: FormatArg,
    /// Built-in validation flavour or `auto`.
    #[arg(long, default_value = "auto", value_parser = parse_flavour_selection)]
    flavour: FlavourSelection,
    /// Custom profile path. Custom profile loading is not available in M0.
    #[arg(long, value_name = "PATH", conflicts_with = "flavour")]
    profile: Option<PathBuf>,
    /// Maximum failed assertion details retained per rule. Use -1 for no practical cap.
    #[arg(long, allow_hyphen_values = true, default_value = "1", value_parser = parse_max_failures)]
    max_failures: MaxDisplayedFailures,
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
}

impl From<FormatArg> for ReportFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Json => Self::Json,
            FormatArg::JsonPretty => Self::JsonPretty,
            FormatArg::Text => Self::Text,
        }
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
    }
}

fn run_validate(args: &ValidateArgs) -> Result<CliExit> {
    let started = Instant::now();
    let format = ReportFormat::from(args.format);
    reject_m0_custom_profile(args.profile.as_ref())?;
    let options = validation_options(args);
    let validator = Validator::new(options).context("failed to initialize validator")?;
    let reports = args
        .paths
        .iter()
        .map(|path| {
            validator
                .validate_path(path)
                .with_context(|| format!("failed to validate {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let exit = reports
        .iter()
        .map(|report| CliExit::from_status(report.status))
        .fold(CliExit::Valid, CliExit::worst);

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if reports.len() == 1 {
        let Some(report) = reports.first() else {
            return Err(anyhow::anyhow!("validation produced no reports"));
        };
        format
            .write_report(report, &mut handle)
            .context("failed to write validation report")?;
    } else {
        let batch = BatchReport::from_items(reports, Vec::new(), started.elapsed());
        format
            .write_batch(&batch, &mut handle)
            .context("failed to write batch report")?;
    }
    handle.flush().context("failed to flush report output")?;
    Ok(exit)
}

fn reject_m0_custom_profile(profile: Option<&PathBuf>) -> Result<()> {
    if let Some(path) = profile {
        return Err(
            PdfvError::Configuration(pdfv_core::ConfigError::InvalidValue {
                field: "profile",
                reason: pdfv_core::BoundedText::new(
                    format!(
                        "custom profile loading is not available in M0: {}",
                        path.display()
                    ),
                    512,
                )?,
            })
            .into(),
        );
    }
    Ok(())
}

fn validation_options(args: &ValidateArgs) -> ValidationOptions {
    let flavour = args.profile.as_ref().map_or_else(
        || args.flavour.clone(),
        |profile_path| FlavourSelection::CustomProfile {
            profile_path: profile_path.clone(),
        },
    );
    ValidationOptions::builder()
        .flavour(flavour)
        .max_failed_assertions_per_rule(args.max_failures)
        .build()
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
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Valid => EXIT_VALID,
            Self::Invalid => EXIT_INVALID,
            Self::ParseFailed => EXIT_PARSE_FAILED,
            Self::Encrypted => EXIT_ENCRYPTED,
            Self::Incomplete => EXIT_INCOMPLETE,
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
