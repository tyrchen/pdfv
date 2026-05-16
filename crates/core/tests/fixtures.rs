use std::{error::Error, io::Cursor};

use pdfv_core::{InputName, ValidationStatus, Validator};

const MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const LEADING_BYTES_INVALID: &[u8] =
    include_bytes!("../../../tests/fixtures/leading-bytes-invalid.pdf");
const NOT_A_PDF: &[u8] = include_bytes!("../../../tests/fixtures/not-a-pdf.pdf");

#[test]
fn test_should_validate_shared_valid_fixture() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?
        .validate_reader(Cursor::new(MINIMAL_VALID), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Valid);
    Ok(())
}

#[test]
fn test_should_validate_shared_invalid_fixture() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?
        .validate_reader(Cursor::new(LEADING_BYTES_INVALID), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert!(
        report
            .profile_reports
            .iter()
            .any(|profile| profile.failed_rules > 0)
    );
    Ok(())
}

#[test]
fn test_should_report_shared_parse_failure_fixture() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?
        .validate_reader(Cursor::new(NOT_A_PDF), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::ParseFailed);
    Ok(())
}
