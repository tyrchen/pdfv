use std::{error::Error, io::Cursor};

use pdfv_core::{
    FlavourSelection, InputName, ParseFact, ValidationOptions, ValidationStatus, Validator, XmpFact,
};

const MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const LEADING_BYTES_INVALID: &[u8] =
    include_bytes!("../../../tests/fixtures/leading-bytes-invalid.pdf");
const NOT_A_PDF: &[u8] = include_bytes!("../../../tests/fixtures/not-a-pdf.pdf");
const XREF_STREAM_OBJECT_STREAM_VALID: &[u8] =
    include_bytes!("../../../tests/fixtures/xref-stream-object-stream-valid.pdf");

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

#[test]
fn test_should_validate_shared_m1_xref_and_object_stream_fixture() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?.validate_reader(
        Cursor::new(XREF_STREAM_OBJECT_STREAM_VALID),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Valid);
    Ok(())
}

#[test]
fn test_should_return_incomplete_for_auto_without_xmp_and_without_default()
-> Result<(), Box<dyn Error>> {
    let options = ValidationOptions::builder()
        .flavour(FlavourSelection::Auto { default: None })
        .build();
    let report = Validator::new(options)?
        .validate_reader(Cursor::new(MINIMAL_VALID), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Incomplete);
    assert!(report.profile_reports.is_empty());
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::AutoDetection { message }
            if message.as_str() == "catalog metadata stream is missing"
    )));
    Ok(())
}

#[test]
fn test_should_auto_select_profile_from_pdfa_xmp_claim() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(pdfa_xmp("2", "B"))),
        InputName::memory(),
    )?;

    assert_eq!(
        report
            .profile_reports
            .first()
            .map(|profile| profile.profile.id.as_str()),
        Some("verapdf-pdfa-2b")
    );
    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Xmp {
            fact:
                XmpFact::FlavourClaim {
                    display_flavour,
                    ..
                },
            ..
        } if display_flavour.as_str() == "pdfa-2b"
    )));
    Ok(())
}

#[test]
fn test_should_auto_select_multiple_profiles_from_xmp_claims() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     xmlns:pdfuaid="http://www.aiim.org/pdfua/ns/id/"
                     xmlns:pdfd="http://pdfa.org/declarations/"
                     pdfaid:part="4"
                     pdfuaid:part="2">
      <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#accessibility1.0</pdfd:conformsTo>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#,
        )),
        InputName::memory(),
    )?;
    let profile_ids = report
        .profile_reports
        .iter()
        .map(|profile| profile.profile.id.as_str())
        .collect::<Vec<_>>();

    assert!(profile_ids.contains(&"verapdf-pdfa-4"));
    assert!(profile_ids.contains(&"verapdf-pdfua-2-iso32005"));
    assert!(profile_ids.contains(&"verapdf-wtpdf-1-0-accessibility"));
    Ok(())
}

#[test]
fn test_should_warn_and_fallback_for_malformed_xmp() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata("<x:xmpmeta><rdf:RDF>")),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Incomplete);
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::AutoDetection { message }
            if message.as_str().contains("invalid profile XML")
    )));
    Ok(())
}

#[test]
fn test_should_warn_for_incompatible_xmp_claim() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(pdfa_xmp("9", "B"))),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Incomplete);
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::IncompatibleProfile { profile_id, .. }
            if profile_id.as_str() == "pdfa-9b"
    )));
    Ok(())
}

#[test]
fn test_should_reject_xmp_doctype_without_external_resource_path() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(
            r#"<!DOCTYPE x [ <!ENTITY ext SYSTEM "file:///etc/passwd"> ]>
<x:xmpmeta xmlns:x="adobe:ns:meta/">&ext;</x:xmpmeta>"#,
        )),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Incomplete);
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::AutoDetection { message }
            if message.as_str().contains("DTD")
                || message.as_str().contains("entity")
                || message.as_str().contains("forbidden")
    )));
    Ok(())
}

fn pdfa_xmp(part: &str, conformance: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="{part}"
                     pdfaid:conformance="{conformance}"/>
  </rdf:RDF>
</x:xmpmeta>"#
    )
}

fn pdf_with_metadata(xmp: impl AsRef<str>) -> Vec<u8> {
    let xmp = xmp.as_ref();
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    push_object(
        &mut bytes,
        &mut offsets,
        1,
        "<< /Type /Catalog /Metadata 2 0 R /Pages 3 0 R >>",
    );
    push_stream_object(&mut bytes, &mut offsets, 2, xmp.as_bytes());
    push_object(
        &mut bytes,
        &mut offsets,
        3,
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
    );
    push_object(
        &mut bytes,
        &mut offsets,
        4,
        "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 1 1] >>",
    );
    let xref_offset = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Root 1 0 R /Size 5 >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn push_object(bytes: &mut Vec<u8>, offsets: &mut Vec<usize>, number: usize, body: &str) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
}

fn push_stream_object(bytes: &mut Vec<u8>, offsets: &mut Vec<usize>, number: usize, stream: &[u8]) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(
        format!("{number} 0 obj\n<< /Length {} >>\nstream\n", stream.len()).as_bytes(),
    );
    bytes.extend_from_slice(stream);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
}
