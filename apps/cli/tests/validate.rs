#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "CLI integration tests create local fixture files synchronously before invoking the \
              binary"
)]

use std::{error::Error, fs::File, io::Write, path::Path};

use assert_cmd::Command;
use md5::{Digest, Md5};
use predicates::{Predicate, str::contains};
use rc4::{KeyInit, Rc4, StreamCipher};
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
fn test_should_list_profiles() -> Result<(), Box<dyn Error>> {
    let output = Command::cargo_bin("pdfv")?
        .args(["profiles", "list"])
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(contains("pdfv-m4").eval(&stdout));
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
