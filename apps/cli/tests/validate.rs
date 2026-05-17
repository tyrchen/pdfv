#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "CLI integration tests create local fixture files synchronously before invoking the \
              binary"
)]

use std::{error::Error, fs::File, io::Write, path::Path};

use aes::{
    Aes128, Aes256,
    cipher::{
        BlockCipherEncrypt, BlockModeEncrypt, KeyInit as AesKeyInit, KeyIvInit,
        block_padding::{NoPadding, Pkcs7},
    },
};
use assert_cmd::Command;
use md5::{Digest, Md5};
use predicates::{Predicate, str::contains};
use rc4::{Rc4, StreamCipher};
use sha2::{Sha256, Sha384, Sha512};
use tempfile::tempdir;

const MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const LEADING_BYTES_INVALID: &[u8] =
    include_bytes!("../../../tests/fixtures/leading-bytes-invalid.pdf");
const NOT_A_PDF: &[u8] = include_bytes!("../../../tests/fixtures/not-a-pdf.pdf");
const PASSWORD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];
const DOCUMENT_ID: &[u8] = b"pdfv-cli-rc4-doc";
const REVISION_6_HASH_MAX_ROUNDS: u16 = 288;

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

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
    assert!(contains("profiles: pdfv-m4").eval(&stdout));
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
fn test_should_validate_pdf_and_emit_raw_xml_report() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "raw"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<rawReport").eval(&stdout));
    assert!(contains("<processorConfig").eval(&stdout));
    assert!(contains("<processorResult").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_accept_raw_format_from_config() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let config = temp.path().join("pdfv.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(&config, b"output:\n  format: raw\n")?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--config"])
        .arg(&config)
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<rawReport").eval(&stdout));
    assert!(contains(r#"<processorConfig tasks="validation"></processorConfig>"#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_validate_pdf_and_emit_html_report() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "html"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<!doctype html>").eval(&stdout));
    assert!(contains("<h1>Validation Report</h1>").eval(&stdout));
    assert!(contains("valid.pdf").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_extract_feature_report_to_json() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--extract", "catalog,page"])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""featureReport""#).eval(&stdout));
    assert!(contains(r#""selectedFamilies":["catalog","page"]"#).eval(&stdout));
    assert!(contains(r#""family":"catalog""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_repair_metadata_to_prefixed_output_without_modifying_input()
-> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let input = temp.path().join("valid.pdf");
    let output_dir = temp.path().join("out");
    std::fs::create_dir(&output_dir)?;
    write_fixture(&input, MINIMAL_VALID)?;
    let before = std::fs::read(&input)?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "repair-metadata",
            "--output-dir",
            output_dir.to_str().ok_or("output dir path must be UTF-8")?,
            "--prefix",
            "fixed-",
            "--format",
            "json",
        ])
        .arg(&input)
        .output()?;

    assert!(output.status.success());
    let repaired = output_dir.join("fixed-valid.pdf");
    assert_eq!(std::fs::read(&input)?, before);
    assert_eq!(std::fs::read(&repaired)?, before);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"noAction""#).eval(&stdout));
    assert!(contains(r#""kind":"copiedUnchanged""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_refuse_metadata_repair_for_parse_failure_and_remove_output()
-> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let input = temp.path().join("not-a-pdf.pdf");
    let output_dir = temp.path().join("out");
    std::fs::create_dir(&output_dir)?;
    write_fixture(&input, NOT_A_PDF)?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "repair-metadata",
            "--output-dir",
            output_dir.to_str().ok_or("output dir path must be UTF-8")?,
            "--format",
            "raw",
        ])
        .arg(&input)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    assert!(!output_dir.join("not-a-pdf.pdf").exists());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<rawRepairReport").eval(&stdout));
    assert!(contains(r#"<refusal kind="parseFailed">"#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_reject_repair_output_directory_same_as_input_parent() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let input = temp.path().join("valid.pdf");
    write_fixture(&input, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "repair-metadata",
            "--output-dir",
            temp.path().to_str().ok_or("temp path must be UTF-8")?,
        ])
        .arg(&input)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""kind":"outputWouldModifyInput""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_refuse_repair_when_output_exists_and_preserve_existing_file()
-> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let input = temp.path().join("valid.pdf");
    let output_dir = temp.path().join("out");
    let output_path = output_dir.join("valid.pdf");
    std::fs::create_dir(&output_dir)?;
    write_fixture(&input, MINIMAL_VALID)?;
    write_fixture(&output_path, b"existing output")?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "repair-metadata",
            "--output-dir",
            output_dir.to_str().ok_or("output dir path must be UTF-8")?,
            "--format",
            "json",
        ])
        .arg(&input)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read(&output_path)?, b"existing output");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""kind":"invalidOutputPath""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_merge_feature_and_policy_reports_to_xml() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let policy = temp.path().join("policy.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &policy,
        b"name: catalog-policy\nrules:\n  - id: catalog-has-no-metadata\n    description: Catalog metadata is absent\n    family: catalog\n    field: hasMetadata\n    operator: equals\n    value:\n      type: bool\n      value: false\n",
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "xml", "--policy-file"])
        .arg(&policy)
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("<featureReport").eval(&stdout));
    assert!(contains(r#"<policyReport name="catalog-policy" isCompliant="true">"#).eval(&stdout));
    assert!(contains(r#"<rule id="catalog-has-no-metadata" passed="true""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_return_invalid_when_policy_fails() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let policy = temp.path().join("policy.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &policy,
        b"name: failing-catalog-policy\nrules:\n  - id: catalog-requires-metadata\n    description: Catalog metadata is required\n    family: catalog\n    field: hasMetadata\n    operator: equals\n    value:\n      type: bool\n      value: true\n",
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--policy-file"])
        .arg(&policy)
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""policyReport""#).eval(&stdout));
    assert!(contains(r#""isCompliant":false"#).eval(&stdout));
    assert!(contains(r#""status":"invalid""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_reject_unknown_feature_family() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--extract", "missingFamily"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("unknown feature family").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_reject_invalid_policy_schema_as_usage() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let policy = temp.path().join("policy.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &policy,
        b"name: invalid-policy\nrules:\n  - id: typo-passes\n    description: Field typo must not pass absent\n    family: catalog\n    field: missingField\n    operator: absent\n",
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--policy-file"])
        .arg(&policy)
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("unknown policy feature field").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_reject_oversized_policy_file() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let policy = temp.path().join("policy.yaml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(&policy, "x".repeat(1024 * 1024 + 1).as_bytes())?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--policy-file"])
        .arg(&policy)
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("policyFile").eval(&stderr));
    assert!(contains("byte limit").eval(&stderr));
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
fn test_should_use_default_flavour_when_auto_detection_has_no_claims() -> Result<(), Box<dyn Error>>
{
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "validate",
            "--format",
            "json",
            "--flavour",
            "auto",
            "--default-flavour",
            "pdfa-2b",
        ])
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""family":"pdfa""#).eval(&stdout));
    assert!(contains(r#""part":2"#).eval(&stdout));
    assert!(contains(r#""conformance":"b""#).eval(&stdout));
    Ok(())
}

#[test]
fn test_should_discover_non_pdf_extension_inputs_when_requested() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let nested = temp.path().join("nested");
    std::fs::create_dir(&nested)?;
    let valid = nested.join("valid.dat");
    write_fixture(&valid, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args([
            "validate",
            "--recursive",
            "--non-pdf-extension",
            "--format",
            "json",
        ])
        .arg(temp.path())
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""totalFiles":1"#).eval(&stdout));
    assert!(contains("valid.dat").eval(&stdout));
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
fn test_should_validate_encrypted_pdf_with_password_file() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("encrypted.pdf");
    let password = temp.path().join("password.txt");
    write_fixture(&path, &encrypted_rc4_fixture()?)?;
    write_fixture(&password, b"user\n")?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--password-file"])
        .arg(&password)
        .arg(&path)
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"valid""#).eval(&stdout));
    assert!(!contains("user").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_exit_encrypted_for_wrong_password_env() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("encrypted.pdf");
    write_fixture(&path, &encrypted_rc4_fixture()?)?;

    let output = Command::cargo_bin("pdfv")?
        .env("PDFV_TEST_PASSWORD", "wrong")
        .args([
            "validate",
            "--format",
            "json",
            "--password-env",
            "PDFV_TEST_PASSWORD",
        ])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"encrypted""#).eval(&stdout));
    assert!(contains("incorrect password").eval(&stdout));
    assert!(!contains("wrong").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_exit_encrypted_for_wrong_aesv3_password_env() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("aesv3-encrypted.pdf");
    write_fixture(&path, &encrypted_aesv3_fixture()?)?;

    let output = Command::cargo_bin("pdfv")?
        .env("PDFV_TEST_PASSWORD", "wrong")
        .args([
            "validate",
            "--format",
            "json",
            "--password-env",
            "PDFV_TEST_PASSWORD",
        ])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"encrypted""#).eval(&stdout));
    assert!(contains("incorrect password").eval(&stdout));
    assert!(!contains("wrong").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_validate_encrypted_pdf_with_password_stdin() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("encrypted.pdf");
    write_fixture(&path, &encrypted_rc4_fixture()?)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--format", "json", "--password-stdin"])
        .arg(&path)
        .write_stdin("user\n")
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains(r#""status":"valid""#).eval(&stdout));
    assert!(!contains("user").eval(&stdout));
    Ok(())
}

#[test]
fn test_should_reject_oversized_password_stdin() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("encrypted.pdf");
    write_fixture(&path, &encrypted_rc4_fixture()?)?;
    let oversized = "x".repeat(1025);

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--password-stdin"])
        .arg(&path)
        .write_stdin(oversized)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("passwordStdin").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_reject_multiple_password_sources() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args([
            "validate",
            "--password-stdin",
            "--password-env",
            "PDFV_TEST_PASSWORD",
            "missing.pdf",
        ])
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    Ok(())
}

#[test]
fn test_should_reject_literal_password_argument_without_echoing_secret()
-> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--password", "secret-value"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("--password").eval(&stderr));
    assert!(!contains("secret-value").eval(&stderr));
    Ok(())
}

fn encrypted_rc4_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    let owner_key = owner_key(b"owner");
    let owner_entry = rc4_crypt(&owner_key, &padded_password(b"user"))?;
    let file_key = file_key(b"user", &owner_entry);
    let user_entry = rc4_crypt(&file_key, &PASSWORD_PADDING)?;
    let title = encrypt_object(&file_key, 1, b"secret-title")?;
    let encrypt_dictionary = format!(
        "<< /Filter /Standard /V 1 /R 2 /Length 40 /O <{}> /U <{}> /P -4 >>",
        hex(&owner_entry),
        hex(&user_entry),
    );
    Ok(pdf_bytes(&title, &encrypt_dictionary))
}

fn encrypted_aesv3_fixture() -> Result<Vec<u8>, Box<dyn Error>> {
    let file_key: Vec<u8> = (0_u8..32).map(|byte| byte.wrapping_add(0x31)).collect();
    let user_validation_salt = b"uvsalt01";
    let user_key_salt = b"uksalt01";
    let owner_validation_salt = b"ovsalt01";
    let owner_key_salt = b"oksalt01";
    let mut user_entry = revision_6_hash(b"user", user_validation_salt, None)?;
    user_entry.extend_from_slice(user_validation_salt);
    user_entry.extend_from_slice(user_key_salt);
    let user_file_key_hash = revision_6_hash(b"user", user_key_salt, None)?;
    let mut owner_entry = revision_6_hash(b"owner", owner_validation_salt, Some(&user_entry))?;
    owner_entry.extend_from_slice(owner_validation_salt);
    owner_entry.extend_from_slice(owner_key_salt);
    let owner_file_key_hash = revision_6_hash(b"owner", owner_key_salt, Some(&user_entry))?;
    let user_encryption_key = aes256_cbc_encrypt_no_padding(&user_file_key_hash, &file_key)?;
    let owner_encryption_key = aes256_cbc_encrypt_no_padding(&owner_file_key_hash, &file_key)?;
    let perms = aes256_block_encrypt(&file_key, &permissions_plaintext())?;
    let title = aes256_encrypt(&file_key, b"secret-title")?;
    let encrypt_dictionary = format!(
        "<< /Filter /Standard /V 5 /R 6 /Length 256 /O <{}> /U <{}> /OE <{}> /UE <{}> /P -4 \
         /Perms <{}> /EncryptMetadata true /CF << /StdCF << /CFM /AESV3 /Length 32 /AuthEvent \
         /DocOpen >> >> /StmF /StdCF /StrF /StdCF >>",
        hex(&owner_entry),
        hex(&user_entry),
        hex(&owner_encryption_key),
        hex(&user_encryption_key),
        hex(&perms),
    );
    Ok(pdf_bytes(&title, &encrypt_dictionary))
}

fn pdf_bytes(title: &[u8], encrypt_dictionary: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0_usize];
    push_object(
        &mut bytes,
        &mut offsets,
        1,
        format!("<< /Type /Catalog /Title <{}> >>", hex(title)).as_bytes(),
    );
    push_object(&mut bytes, &mut offsets, 2, encrypt_dictionary.as_bytes());
    let xref_offset = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Encrypt 2 0 R /Size {} /ID [<{}> <{}>] \
             >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len(),
            hex(DOCUMENT_ID),
            hex(DOCUMENT_ID),
        )
        .as_bytes(),
    );
    bytes
}

fn push_object(bytes: &mut Vec<u8>, offsets: &mut Vec<usize>, number: u32, body: &[u8]) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
}

fn owner_key(password: &[u8]) -> Vec<u8> {
    let mut digest = Md5::digest(padded_password(password)).to_vec();
    digest.truncate(5);
    digest
}

fn file_key(password: &[u8], owner_entry: &[u8]) -> Vec<u8> {
    let mut hasher = Md5::new();
    hasher.update(padded_password(password));
    hasher.update(owner_entry);
    hasher.update((-4_i32).to_le_bytes());
    hasher.update(DOCUMENT_ID);
    let mut digest = hasher.finalize().to_vec();
    digest.truncate(5);
    digest
}

fn encrypt_object(file_key: &[u8], number: u32, bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut hasher = Md5::new();
    let object_number = number.to_le_bytes();
    hasher.update(file_key);
    hasher.update(object_number.get(..3).unwrap_or(&object_number));
    hasher.update([0_u8, 0_u8]);
    let mut key = hasher.finalize().to_vec();
    key.truncate(file_key.len().saturating_add(5).min(16));
    rc4_crypt(&key, bytes)
}

fn aes256_encrypt(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let iv = [0x24_u8; 16];
    let mut buffer = vec![0_u8; bytes.len().saturating_add(16)];
    let ciphertext = Aes256CbcEnc::new_from_slices(key, &iv)
        .map_err(|_| std::io::Error::other("invalid aes key"))?
        .encrypt_padded_b2b::<Pkcs7>(bytes, &mut buffer)
        .map_err(|_| std::io::Error::other("invalid aes padding"))?;
    let mut output = iv.to_vec();
    output.extend_from_slice(ciphertext);
    Ok(output)
}

fn aes256_cbc_encrypt_no_padding(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buffer = bytes.to_vec();
    Aes256CbcEnc::new_from_slices(key, &[0_u8; 16])
        .map_err(|_| std::io::Error::other("invalid aes256 key"))?
        .encrypt_padded::<NoPadding>(&mut buffer, bytes.len())
        .map_err(|_| std::io::Error::other("invalid aes256 plaintext"))?;
    Ok(buffer)
}

fn aes256_block_encrypt(key: &[u8], bytes: &[u8; 16]) -> Result<Vec<u8>, Box<dyn Error>> {
    let cipher =
        Aes256::new_from_slice(key).map_err(|_| std::io::Error::other("invalid aes256 key"))?;
    let mut block = aes::Block::from(*bytes);
    cipher.encrypt_block(&mut block);
    Ok(block.to_vec())
}

fn revision_6_hash(
    password: &[u8],
    salt: &[u8],
    owner_context: Option<&[u8]>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let password = password.get(..password.len().min(127)).unwrap_or(password);
    let mut hasher = Sha256::new();
    hasher.update(password);
    hasher.update(salt);
    if let Some(context) = owner_context {
        hasher.update(context);
    }
    let mut digest = hasher.finalize().to_vec();
    let mut round = 0_u16;
    loop {
        if round >= REVISION_6_HASH_MAX_ROUNDS {
            return Err(std::io::Error::other("r6 hash exceeded bound").into());
        }
        let context_len = owner_context.map_or(0, <[u8]>::len);
        let mut k1 = Vec::with_capacity(password.len() + digest.len() + context_len);
        k1.extend_from_slice(password);
        k1.extend_from_slice(&digest);
        if let Some(context) = owner_context {
            k1.extend_from_slice(context);
        }
        let mut encrypted = Vec::with_capacity(k1.len().saturating_mul(64));
        for _ in 0..64 {
            encrypted.extend_from_slice(&k1);
        }
        let key = digest
            .get(..16)
            .ok_or_else(|| std::io::Error::other("missing r6 key"))?;
        let iv = digest
            .get(16..32)
            .ok_or_else(|| std::io::Error::other("missing r6 iv"))?;
        cbc::Encryptor::<Aes128>::new_from_slices(key, iv)
            .map_err(|_| std::io::Error::other("invalid r6 aes inputs"))?
            .encrypt_padded::<NoPadding>(&mut encrypted, k1.len().saturating_mul(64))
            .map_err(|_| std::io::Error::other("invalid r6 aes plaintext"))?;
        let selector = encrypted
            .get(..16)
            .ok_or_else(|| std::io::Error::other("missing r6 selector"))?
            .iter()
            .fold(0_u16, |sum, byte| sum + u16::from(*byte))
            % 3;
        digest = match selector {
            0 => Sha256::digest(&encrypted).to_vec(),
            1 => Sha384::digest(&encrypted).to_vec(),
            _ => Sha512::digest(&encrypted).to_vec(),
        };
        let last = encrypted
            .last()
            .copied()
            .ok_or_else(|| std::io::Error::other("empty r6 block"))?;
        if round >= 63 && u16::from(last) <= round.saturating_sub(32) {
            break;
        }
        round = round.saturating_add(1);
    }
    digest.truncate(32);
    Ok(digest)
}

fn permissions_plaintext() -> [u8; 16] {
    let mut plaintext = [0_u8; 16];
    if let Some(target) = plaintext.get_mut(..4) {
        target.copy_from_slice(&(-4_i32).to_le_bytes());
    }
    if let Some(target) = plaintext.get_mut(4..8) {
        target.copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    }
    if let Some(target) = plaintext.get_mut(8..12) {
        target.copy_from_slice(b"Tadb");
    }
    if let Some(target) = plaintext.get_mut(12..16) {
        target.copy_from_slice(b"pdfv");
    }
    plaintext
}

fn rc4_crypt(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = bytes.to_vec();
    let mut cipher =
        Rc4::new_from_slice(key).map_err(|_| std::io::Error::other("invalid rc4 key"))?;
    cipher.apply_keystream(&mut output);
    Ok(output)
}

fn padded_password(password: &[u8]) -> [u8; 32] {
    let mut padded = PASSWORD_PADDING;
    let copy_len = password.len().min(32);
    if let (Some(target), Some(source)) = (padded.get_mut(..copy_len), password.get(..copy_len)) {
        target.copy_from_slice(source);
    }
    padded
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(
            DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'),
        ));
        output.push(char::from(
            DIGITS
                .get(usize::from(byte & 0x0f))
                .copied()
                .unwrap_or(b'0'),
        ));
    }
    output
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
fn test_should_reject_default_flavour_with_custom_profile() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    let profile = temp.path().join("profile.xml");
    write_fixture(&path, MINIMAL_VALID)?;
    write_fixture(
        &profile,
        br#"<?xml version="1.0" encoding="UTF-8"?><profile/>"#,
    )?;

    let output = Command::cargo_bin("pdfv")?
        .args(["validate", "--profile"])
        .arg(&profile)
        .args(["--default-flavour", "pdfa-1b"])
        .arg(&path)
        .output()?;

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(contains("cannot be used with").eval(&stderr));
    assert!(contains("--profile").eval(&stderr));
    assert!(contains("--default-flavour").eval(&stderr));
    Ok(())
}

#[test]
fn test_should_list_profiles() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args(["profiles", "list"])
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("pdfv-m4").eval(&stdout));
    assert!(contains("verapdf-pdfa-1b").eval(&stdout));
    assert!(contains("verapdf-pdfua-2-iso32005").eval(&stdout));
    assert!(contains("acfcc419a5df444e3e8b2a18266d01e249299957").eval(&stdout));
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 16);
    for line in lines {
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 8);
        assert!(columns.get(2).is_some_and(|value| value.ends_with('%')));
    }
    Ok(())
}

#[test]
fn test_should_validate_with_every_phase_13_builtin_profile() -> Result<(), Box<dyn Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("valid.pdf");
    write_fixture(&path, MINIMAL_VALID)?;

    for (flavour, profile_id) in [
        ("pdfa-1a", "verapdf-pdfa-1a"),
        ("pdfa-1b", "verapdf-pdfa-1b"),
        ("pdfa-2a", "verapdf-pdfa-2a"),
        ("pdfa-2b", "verapdf-pdfa-2b"),
        ("pdfa-2u", "verapdf-pdfa-2u"),
        ("pdfa-3a", "verapdf-pdfa-3a"),
        ("pdfa-3b", "verapdf-pdfa-3b"),
        ("pdfa-3u", "verapdf-pdfa-3u"),
        ("pdfa-4", "verapdf-pdfa-4"),
        ("pdfa-4e", "verapdf-pdfa-4e"),
        ("pdfa-4f", "verapdf-pdfa-4f"),
        ("pdfua-1", "verapdf-pdfua-1"),
        ("pdfua-2-iso32005", "verapdf-pdfua-2-iso32005"),
        ("wtpdf-1-0-accessibility", "verapdf-wtpdf-1-0-accessibility"),
        ("wtpdf-1-0-reuse", "verapdf-wtpdf-1-0-reuse"),
    ] {
        let output = Command::cargo_bin("pdfv")?
            .args(["validate", "--format", "json", "--flavour", flavour])
            .arg(&path)
            .output()?;

        assert_eq!(output.status.code(), Some(4), "{flavour}");
        let stdout = String::from_utf8(output.stdout)?;
        assert!(contains(format!(r#""id":"{profile_id}""#)).eval(&stdout));
        assert!(contains(r#""status":"incomplete""#).eval(&stdout));
    }
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
