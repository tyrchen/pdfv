#![allow(
    clippy::disallowed_types,
    reason = "CLI integration tests create local fixture files synchronously before invoking the \
              binary"
)]

use std::{error::Error, fs::File, io::Write, path::Path};

use assert_cmd::Command;
use predicates::{Predicate, str::contains};
use tempfile::tempdir;

const MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const LEADING_BYTES_INVALID: &[u8] =
    include_bytes!("../../../tests/fixtures/leading-bytes-invalid.pdf");
const NOT_A_PDF: &[u8] = include_bytes!("../../../tests/fixtures/not-a-pdf.pdf");

fn write_fixture(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    Ok(())
}

#[test]
fn test_should_validate_pdf_and_emit_text_report() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "text"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("valid.pdf: valid").eval(&stdout));
    assert!(contains("profiles: pdfv-m0").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_exit_invalid_for_failed_validation() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("invalid.pdf");
    write_fixture(&path, LEADING_BYTES_INVALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json-pretty", "--max-failures", "1"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status": "invalid""#).eval(&stdout));
    assert!(contains("m0-header-offset-zero").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_exit_parse_failed_for_non_pdf() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("not-a-pdf.pdf");
    write_fixture(&path, NOT_A_PDF)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"parseFailed""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_emit_batch_report_for_multiple_inputs() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let valid = temp.path().join("valid.pdf");
    let invalid = temp.path().join("invalid.pdf");
    write_fixture(&valid, MINIMAL_VALID)?;
    write_fixture(&invalid, LEADING_BYTES_INVALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json"])
        .arg(&valid)
        .arg(&invalid)
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""totalFiles":2"#).eval(&stdout));
    assert!(contains(r#""valid":1"#).eval(&stdout));
    assert!(contains(r#""invalid":1"#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_accept_unbounded_max_failures_flag() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("invalid.pdf");
    write_fixture(&path, LEADING_BYTES_INVALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--max-failures", "-1"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"invalid""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_exit_usage_for_custom_profile_in_m0() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let profile = temp.path().join("profile.xml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(&profile, b"<profile />")?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--profile"])
        .arg(&profile)
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("custom profile loading is not available in M0").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_exit_incomplete_for_unsupported_flavour() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--flavour", "pdfa-999z"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("unsupported profile selection").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_exit_usage_for_invalid_max_failures() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--max-failures", "0", "missing.pdf"])
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("max failures must be -1").eval(&stderr));
    Ok(())
}
