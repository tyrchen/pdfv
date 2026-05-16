#![allow(
    clippy::disallowed_methods,
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
fn test_should_validate_pdf_and_emit_xml_report() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "xml"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#"<?xml version="1.0" encoding="utf-8"?>"#).eval(&stdout));
    assert!(contains("<validationReport").eval(&stdout));
    assert!(contains(r#"isCompliant="true""#).eval(&stdout));
    assert!(contains(r#"<batchSummary totalJobs="1""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_accept_mrr_as_deprecated_xml_alias() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "mrr"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<report>").eval(&stdout));
    assert!(contains("<validationReport").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_discover_recursive_inputs_with_bounded_jobs() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let nested = temp.path().join("nested");
    std::fs::create_dir(&nested)?;
    let valid = nested.join("valid.pdf");
    write_fixture(&valid, MINIMAL_VALID)?;
    write_fixture(&nested.join("notes.txt"), b"not a pdf")?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--recursive", "--jobs", "2", "--format", "json"])
        .arg(temp.path())
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""totalFiles":1"#).eval(&stdout));
    assert!(contains("valid.pdf").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_write_output_file_and_redact_paths_from_config() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let config = temp.path().join("pdfv.yaml");
    let report = temp.path().join("report.json");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &config,
        format!(
            "output:\n  format: json\n  path: {}\n  redactPaths: true\n",
            report.display()
        )
        .as_bytes(),
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--config"])
        .arg(&config)
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let contents = std::fs::read_to_string(report)?;
    assert!(contains(r#""status":"valid""#).eval(&contents));
    assert!(!contains("valid.pdf").eval(&contents));
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
fn test_should_reject_jobs_above_compiled_cap() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--jobs", "257", "missing.pdf"])
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("jobs must be in 1..=256").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_reject_config_resource_limits_above_hard_cap() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let config = temp.path().join("pdfv.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &config,
        b"resources:\n  maxFileBytes: 1099511627776\n  maxObjects: 1000000\n  maxObjectDepth: 128\n  maxArrayLen: 65536\n  maxDictEntries: 16384\n  maxNameBytes: 127\n  maxStringBytes: 1048576\n  maxStreamDeclaredBytes: 134217728\n  maxStreamDecodeBytes: 268435456\n  maxParseFacts: 100000\n",
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--config"])
        .arg(&config)
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("maxFileBytes").eval(&stderr));
    assert!(contains("hard cap").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_continue_batch_after_internal_file_error() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let valid = temp.path().join("valid.pdf");
    let missing = temp.path().join("missing.pdf");
    write_fixture(&valid, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json"])
        .arg(&valid)
        .arg(&missing)
        .output()?;

    assert_eq!(output.status.code(), Some(70));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""totalFiles":2"#).eval(&stdout));
    assert!(contains(r#""valid":1"#).eval(&stdout));
    assert!(contains(r#""internalErrors":1"#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_validate_with_custom_profile_xml() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let profile = temp.path().join("profile.xml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &profile,
        br#"<?xml version="1.0" encoding="UTF-8"?>
<profile flavour="PDFA_1_B">
  <details><name>Custom smoke profile</name></details>
  <rules>
    <rule object="CosDocument">
      <id specification="LOCAL" clause="1" testNumber="1"/>
      <description>Catalog must be present</description>
      <test>hasCatalog == true</test>
      <error><message>Catalog is missing</message></error>
    </rule>
  </rules>
</profile>"#,
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--profile"])
        .arg(&profile)
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""id":"verapdf-pdfa-1b""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_list_profiles() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args(["profiles", "list"])
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("pdfv-m0").eval(&stdout));
    assert!(contains("verapdf-pdfa-1b").eval(&stdout));
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
