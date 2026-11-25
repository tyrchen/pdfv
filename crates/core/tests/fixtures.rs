use std::{collections::BTreeSet, error::Error, io::Cursor, num::NonZeroU32};

use pdfv_core::{
    Assertion, FlavourSelection, InputName, ParseFact, ResourceLimits, ValidationFlavour,
    ValidationOptions, ValidationStatus, ValidationWarning, Validator, XmpFact,
};

const MINIMAL_VALID: &[u8] = include_bytes!("fixtures/minimal-valid.pdf");
const LEADING_BYTES_INVALID: &[u8] = include_bytes!("fixtures/leading-bytes-invalid.pdf");
const NOT_A_PDF: &[u8] = include_bytes!("fixtures/not-a-pdf.pdf");
const XREF_STREAM_OBJECT_STREAM_VALID: &[u8] =
    include_bytes!("fixtures/xref-stream-object-stream-valid.pdf");
const UNSUPPORTED_XMP_PART: &[u8] = include_bytes!("fixtures/unknown-xmp-part.pdf");
const FLATE_FIXTURE_DIAGNOSTIC: &str = if cfg!(feature = "flate") {
    "stream decode failed"
} else {
    "unsupported stream filter: FlateDecode"
};

#[cfg(not(feature = "decrypt"))]
#[test]
fn test_should_report_encrypted_with_password_when_decryption_is_disabled()
-> Result<(), Box<dyn Error>> {
    let bytes = include_bytes!("fixtures/legacy-qpdf-r2.pdf");
    let options = ValidationOptions::builder()
        .password(Some(pdfv_core::PasswordSecret::new("leaf")?))
        .build();
    let report =
        Validator::new(options)?.validate_reader(Cursor::new(bytes), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Encrypted);
    assert!(report.profile_reports.is_empty());
    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Encryption {
            decrypted: false,
            ..
        }
    )));
    Ok(())
}

#[test]
fn test_should_reject_minimal_pdf_without_pdfa_metadata() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?
        .validate_reader(Cursor::new(MINIMAL_VALID), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert!(report.profile_reports.iter().any(|profile| {
        profile
            .failed_assertions
            .iter()
            .any(|assertion| assertion.rule_id.0.as_str() == "iso-19005-1-6-7-2-1")
    }));
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
fn test_should_reject_xref_and_object_stream_fixture_under_pdfa_1b() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(pdfv_core::ValidationOptions::default())?.validate_reader(
        Cursor::new(XREF_STREAM_OBJECT_STREAM_VALID),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Invalid);
    Ok(())
}

#[test]
fn test_should_scope_pdfa_page_boundary_rules_to_effective_page_boxes() -> Result<(), Box<dyn Error>>
{
    let bytes = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /OtherNumbers [0 0 1 1] >>
endobj
2 0 obj
<< /Type /Pages /Kids 6 0 R /Count 2 /MediaBox 5 0 R /CropBox [0 0 3 14400]
   /TrimBox [0 0 1 1] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /BleedBox [100 100 0 0] >>
endobj
4 0 obj
<< /Type 7 0 R /Parent 2 0 R /CropBox [0 0 2.5 100] >>
endobj
5 0 obj
[0 0 100 100]
endobj
6 0 obj
[3 0 R 4 0 R]
endobj
7 0 obj
/Page
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
    let options = ValidationOptions::builder()
        .flavour(FlavourSelection::Explicit {
            flavour: ValidationFlavour::new(
                "pdfa",
                NonZeroU32::new(2).ok_or("invalid part")?,
                "b",
            )?,
        })
        .record_passed_assertions(true)
        .build();
    let report =
        Validator::new(options)?.validate_reader(Cursor::new(bytes), InputName::memory())?;
    let profile = report
        .profile_reports
        .first()
        .ok_or("missing PDF/A-2b report")?;
    let rule = "iso-19005-2-6-1-13-11";
    assert!(
        !profile
            .unsupported_rules
            .iter()
            .any(|result| result.rule_id.0.as_str() == rule)
    );
    let contexts = |assertions: &[Assertion]| {
        assertions
            .iter()
            .filter(|assertion| assertion.rule_id.0.as_str() == rule)
            .filter_map(|assertion| assertion.object_context.as_ref())
            .map(|context| context.as_str().to_owned())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        contexts(&profile.passed_assertions),
        [
            "root/page[0]/BleedBox[0]",
            "root/page[0]/CropBox[0]",
            "root/page[0]/MediaBox[0]",
            "root/page[1]/MediaBox[0]"
        ]
        .map(str::to_owned)
        .into_iter()
        .collect()
    );
    assert_eq!(
        contexts(&profile.failed_assertions),
        ["root/page[1]/CropBox[0]".to_owned()].into_iter().collect()
    );
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
      <pdfd:declarations>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#accessibility1.0</pdfd:conformsTo>
      </pdfd:declarations>
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
fn test_should_evaluate_pdfua_identification_from_pdfua_claim() -> Result<(), Box<dyn Error>> {
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
                     pdfaid:part="4"
                     pdfuaid:part="2"
                     pdfuaid:rev="2024"/>
  </rdf:RDF>
</x:xmpmeta>"#,
        )),
        InputName::memory(),
    )?;

    let pdfua_report = report
        .profile_reports
        .iter()
        .find(|profile| profile.profile.id.as_str() == "verapdf-pdfua-2-iso32005")
        .ok_or("missing PDF/UA-2 profile report")?;
    let failed_rule_ids = pdfua_report
        .failed_assertions
        .iter()
        .map(|assertion| assertion.rule_id.0.as_str())
        .collect::<Vec<_>>();

    assert!(!failed_rule_ids.contains(&"ISO_14289_2-5-2"));
    assert!(!failed_rule_ids.contains(&"ISO_14289_2-5-3"));
    assert!(!failed_rule_ids.contains(&"ISO_14289_2-5-4"));
    assert!(!failed_rule_ids.contains(&"ISO_14289_2-5-5"));
    Ok(())
}

#[test]
fn test_should_auto_select_multiple_wtpdf_declaration_profiles() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfd="http://pdfa.org/declarations/">
      <pdfd:declarations>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#reuse1.0</pdfd:conformsTo>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#accessibility1.0</pdfd:conformsTo>
      </pdfd:declarations>
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

    assert!(profile_ids.contains(&"verapdf-wtpdf-1-0-reuse"));
    assert!(profile_ids.contains(&"verapdf-wtpdf-1-0-accessibility"));
    Ok(())
}

#[test]
fn test_should_ignore_wtpdf_conforms_to_outside_declarations() -> Result<(), Box<dyn Error>> {
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Auto { default: None })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfd="http://pdfa.org/declarations/">
      <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#reuse1.0</pdfd:conformsTo>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#,
        )),
        InputName::memory(),
    )?;

    assert_eq!(report.status, ValidationStatus::Incomplete);
    assert!(report.profile_reports.is_empty());
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
    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Xmp {
            fact: XmpFact::Malformed { .. },
            ..
        }
    )));
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
fn test_should_validate_default_pdfa_profile_when_xmp_claim_is_unsupported()
-> Result<(), Box<dyn Error>> {
    let report = Validator::new(ValidationOptions::default())?
        .validate_reader(Cursor::new(UNSUPPORTED_XMP_PART), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert!(report.profile_reports.iter().any(|profile| {
        profile.profile.id.as_str() == "verapdf-pdfa-1b"
            && profile
                .failed_assertions
                .iter()
                .any(|assertion| assertion.rule_id.0.as_str() == "iso-19005-1-6-7-11-2")
    }));
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::IncompatibleProfile { profile_id, .. }
            if profile_id.as_str() == "pdfa-9b"
    )));
    Ok(())
}

#[test]
fn test_should_validate_default_profile_when_metadata_stream_cannot_decode()
-> Result<(), Box<dyn Error>> {
    let bytes = pdf_with_metadata_stream_entries(
        "corrupt deflate stream",
        "/Type /Metadata /Subtype /XML /Filter /FlateDecode",
    );
    let report = Validator::new(ValidationOptions::default())?
        .validate_reader(Cursor::new(bytes), InputName::memory())?;

    assert_eq!(report.status, ValidationStatus::Invalid);
    assert_eq!(
        report
            .profile_reports
            .first()
            .map(|profile| profile.profile.id.as_str()),
        Some("verapdf-pdfa-1b")
    );
    assert!(report.profile_reports.iter().any(|profile| {
        profile
            .failed_assertions
            .iter()
            .any(|assertion| assertion.rule_id.0.as_str() == "iso-19005-1-6-7-2-2")
    }));
    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Xmp { fact: XmpFact::Malformed { reason }, .. }
            if reason.as_str().contains(FLATE_FIXTURE_DIAGNOSTIC)
    )));
    Ok(())
}

#[test]
fn test_should_keep_decode_warning_when_parse_facts_are_capped() -> Result<(), Box<dyn Error>> {
    let mut limits = ResourceLimits::default();
    limits.max_parse_facts = 0;
    let options = ValidationOptions::builder().resource_limits(limits).build();
    let bytes = pdf_with_metadata_stream_entries("corrupt deflate stream", "/Filter /FlateDecode");
    let report =
        Validator::new(options)?.validate_reader(Cursor::new(bytes), InputName::memory())?;

    assert_eq!(report.parse_facts, [] as [ParseFact; 0]);
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        ValidationWarning::AutoDetection { message } if message.as_str().contains(FLATE_FIXTURE_DIAGNOSTIC)
    )));
    Ok(())
}

#[test]
fn test_should_skip_incompatible_xmp_profile_groups() -> Result<(), Box<dyn Error>> {
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
                     pdfaid:part="1"
                     pdfaid:conformance="B"
                     pdfuaid:part="2"/>
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

    assert!(profile_ids.contains(&"verapdf-pdfa-1b"));
    assert!(!profile_ids.contains(&"verapdf-pdfua-2-iso32005"));
    assert!(report.warnings.iter().any(|warning| matches!(
        warning,
        pdfv_core::ValidationWarning::IncompatibleProfile { profile_id, .. }
            if profile_id.as_str() == "verapdf-pdfua-2-iso32005"
    )));
    Ok(())
}

#[test]
fn test_should_expose_xmp_facts_for_explicit_profile_selection() -> Result<(), Box<dyn Error>> {
    let flavour = pdfv_core::ValidationFlavour::new("pdfa", std::num::NonZeroU32::MIN, "b")?;
    let report = Validator::new(
        ValidationOptions::builder()
            .flavour(FlavourSelection::Explicit { flavour })
            .build(),
    )?
    .validate_reader(
        Cursor::new(pdf_with_metadata(pdfa_xmp("1", "B"))),
        InputName::memory(),
    )?;

    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Xmp {
            fact:
                XmpFact::FlavourClaim {
                    display_flavour,
                    ..
                },
            ..
        } if display_flavour.as_str() == "pdfa-1b"
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
    assert!(report.parse_facts.iter().any(|fact| matches!(
        fact,
        ParseFact::Xmp {
            fact: XmpFact::HostileXmlRejected { .. },
            ..
        }
    )));
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
    pdf_with_metadata_stream_entries(xmp.as_ref(), "")
}

fn pdf_with_metadata_stream_entries(xmp: &str, entries: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    push_object(
        &mut bytes,
        &mut offsets,
        1,
        "<< /Type /Catalog /Metadata 2 0 R /Pages 3 0 R >>",
    );
    push_stream_object(&mut bytes, &mut offsets, 2, xmp.as_bytes(), entries);
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

fn push_stream_object(
    bytes: &mut Vec<u8>,
    offsets: &mut Vec<usize>,
    number: usize,
    stream: &[u8],
    entries: &str,
) {
    offsets.push(bytes.len());
    bytes.extend_from_slice(
        format!(
            "{number} 0 obj\n<< /Length {} {entries} >>\nstream\n",
            stream.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(stream);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
}
