//! Deterministic parity metrics and semantic corpus agreement reports.

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::{
    BoundedText, FeatureSelection, FlavourSelection, Identifier, InputName, ObjectTypeName, Result,
    ValidationFlavour, ValidationOptions, ValidationStatus, Validator,
};

const CHECKED_IN_MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const CHECKED_IN_LEADING_BYTES_INVALID: &[u8] =
    include_bytes!("../../../tests/fixtures/leading-bytes-invalid.pdf");
const CHECKED_IN_NOT_A_PDF: &[u8] = include_bytes!("../../../tests/fixtures/not-a-pdf.pdf");
const CHECKED_IN_XREF_STREAM_OBJECT_STREAM_VALID: &[u8] =
    include_bytes!("../../../tests/fixtures/xref-stream-object-stream-valid.pdf");

/// Semantic corpus agreement report for normal, Java-free parity checks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorpusAgreementReport {
    /// Deterministic schema version for this report contract.
    pub schema_version: Identifier,
    /// Whether live veraPDF oracle rows were executed for this report.
    pub live_oracle_enabled: bool,
    /// Summary counters for all rows.
    pub summary: CorpusAgreementSummary,
    /// Semantic corpus rows in deterministic fixture order.
    pub rows: Vec<CorpusAgreementRow>,
}

/// Summary counters for a corpus agreement report.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorpusAgreementSummary {
    /// Total corpus rows.
    pub total_rows: u64,
    /// Rows where pdfv matched the expected semantic outcome.
    pub matches: u64,
    /// Rows with documented expected drift.
    pub expected_drift: u64,
    /// Rows with unexpected drift.
    pub unexpected_drift: u64,
}

/// One semantic corpus agreement row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorpusAgreementRow {
    /// Local fixture path or generated fixture id.
    pub fixture: BoundedText,
    /// Fixture provenance.
    pub source: CorpusFixtureSource,
    /// Expected veraPDF or generated semantic outcome.
    pub vera_pdf_outcome: CorpusOutcome,
    /// Observed pdfv outcome.
    pub pdfv_outcome: CorpusOutcome,
    /// Selected profile or profile policy.
    pub profile: BoundedText,
    /// Agreement classification.
    pub agreement: CorpusAgreement,
    /// Required semantic families covered by this row.
    pub semantic_families: Vec<BoundedText>,
    /// Observed feature families when this row extracts features.
    pub observed_feature_families: Vec<BoundedText>,
    /// Reason for non-matching rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BoundedText>,
}

/// Corpus fixture source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum CorpusFixtureSource {
    /// Generated in-process fixture.
    Generated,
    /// Checked-in fixture with manifest provenance.
    CheckedIn,
}

/// Semantic validation outcome used for corpus comparison.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum CorpusOutcome {
    /// All required checks passed.
    Valid,
    /// One or more required checks failed.
    Invalid,
    /// Validation stopped because the input is encrypted.
    Encrypted,
    /// Validation could not complete due unsupported required checks.
    Incomplete,
    /// Input could not be parsed.
    ParseFailed,
}

impl From<ValidationStatus> for CorpusOutcome {
    fn from(value: ValidationStatus) -> Self {
        match value {
            ValidationStatus::Valid => Self::Valid,
            ValidationStatus::Invalid => Self::Invalid,
            ValidationStatus::Encrypted => Self::Encrypted,
            ValidationStatus::Incomplete => Self::Incomplete,
            ValidationStatus::ParseFailed => Self::ParseFailed,
        }
    }
}

/// Corpus row agreement classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum CorpusAgreement {
    /// pdfv matched the expected semantic outcome.
    Match,
    /// pdfv differed in a documented way.
    ExpectedDrift,
    /// pdfv differed unexpectedly.
    UnexpectedDrift,
}

/// Builds the deterministic Java-free corpus agreement report.
///
/// # Errors
///
/// Returns [`crate::PdfvError`] if validation or bounded report construction fails.
pub fn corpus_agreement_report() -> Result<CorpusAgreementReport> {
    let mut rows = checked_in_corpus_rows()?;
    rows.extend(generated_corpus_rows()?);
    let summary = CorpusAgreementSummary::from_rows(&rows);
    Ok(CorpusAgreementReport {
        schema_version: Identifier::new("phase19-corpus-agreement-v1")?,
        live_oracle_enabled: false,
        summary,
        rows,
    })
}

fn checked_in_corpus_rows() -> Result<Vec<CorpusAgreementRow>> {
    Ok(vec![
        corpus_row(CorpusRowSpec {
            fixture: "tests/fixtures/minimal-valid.pdf",
            source: CorpusFixtureSource::CheckedIn,
            bytes: CHECKED_IN_MINIMAL_VALID,
            options: default_options(),
            expected: CorpusOutcome::Valid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["parser"],
            required_feature_families: &[],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "tests/fixtures/leading-bytes-invalid.pdf",
            source: CorpusFixtureSource::CheckedIn,
            bytes: CHECKED_IN_LEADING_BYTES_INVALID,
            options: default_options(),
            expected: CorpusOutcome::Invalid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["parser"],
            required_feature_families: &[],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "tests/fixtures/not-a-pdf.pdf",
            source: CorpusFixtureSource::CheckedIn,
            bytes: CHECKED_IN_NOT_A_PDF,
            options: default_options(),
            expected: CorpusOutcome::ParseFailed,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["parser"],
            required_feature_families: &[],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "tests/fixtures/xref-stream-object-stream-valid.pdf",
            source: CorpusFixtureSource::CheckedIn,
            bytes: CHECKED_IN_XREF_STREAM_OBJECT_STREAM_VALID,
            options: default_options(),
            expected: CorpusOutcome::Valid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["parser"],
            required_feature_families: &[],
        })?,
    ])
}

fn generated_corpus_rows() -> Result<Vec<CorpusAgreementRow>> {
    Ok(vec![
        corpus_row(CorpusRowSpec {
            fixture: "generated:profile-official-pdfa-1b",
            source: CorpusFixtureSource::Generated,
            bytes: minimal_pdf(),
            options: explicit_profile_options("pdfa", 1, "b")?,
            expected: CorpusOutcome::Incomplete,
            profile: "pdfa-1b",
            semantic_families: &["profile"],
            required_feature_families: &[],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "generated:operator-content-stream",
            source: CorpusFixtureSource::Generated,
            bytes: &content_operator_pdf(),
            options: feature_options(),
            expected: CorpusOutcome::Valid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["operator"],
            required_feature_families: &[
                "contentStream",
                "operator",
                "markedContent",
                "inlineImage",
            ],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "generated:resource-font-color",
            source: CorpusFixtureSource::Generated,
            bytes: &resource_font_color_pdf(),
            options: feature_options(),
            expected: CorpusOutcome::Valid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["resource", "font", "color"],
            required_feature_families: &["resourceUse", "font", "colorSpace", "outputIntent"],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "generated:xmp-pdfa-claim",
            source: CorpusFixtureSource::Generated,
            bytes: &xmp_pdf(),
            options: default_options(),
            expected: CorpusOutcome::Incomplete,
            profile: "auto-detected-pdfa-2b",
            semantic_families: &["xmp"],
            required_feature_families: &[],
        })?,
        corpus_row(CorpusRowSpec {
            fixture: "generated:accessibility-structure",
            source: CorpusFixtureSource::Generated,
            bytes: &accessibility_pdf(),
            options: feature_options(),
            expected: CorpusOutcome::Valid,
            profile: "auto-default-pdfa-1b",
            semantic_families: &["accessibility"],
            required_feature_families: &[
                "accessibilityDocument",
                "structureElement",
                "textChunk",
                "imageChunk",
                "accessibilityAnnotation",
                "artifact",
                "heading",
                "list",
                "link",
            ],
        })?,
    ])
}

impl CorpusAgreementSummary {
    fn from_rows(rows: &[CorpusAgreementRow]) -> Self {
        let mut summary = Self {
            total_rows: u64::try_from(rows.len()).unwrap_or(u64::MAX),
            ..Self::default()
        };
        for row in rows {
            match row.agreement {
                CorpusAgreement::Match => summary.matches = summary.matches.saturating_add(1),
                CorpusAgreement::ExpectedDrift => {
                    summary.expected_drift = summary.expected_drift.saturating_add(1);
                }
                CorpusAgreement::UnexpectedDrift => {
                    summary.unexpected_drift = summary.unexpected_drift.saturating_add(1);
                }
            }
        }
        summary
    }
}

#[derive(Debug)]
struct CorpusRowSpec<'a> {
    fixture: &'a str,
    source: CorpusFixtureSource,
    bytes: &'a [u8],
    options: ValidationOptions,
    expected: CorpusOutcome,
    profile: &'a str,
    semantic_families: &'a [&'a str],
    required_feature_families: &'a [&'a str],
}

fn corpus_row(spec: CorpusRowSpec<'_>) -> Result<CorpusAgreementRow> {
    let report = Validator::new(spec.options)?
        .validate_reader(Cursor::new(spec.bytes), InputName::memory())?;
    let observed = CorpusOutcome::from(report.status);
    let observed_feature_families = report
        .feature_report
        .map(|features| {
            features
                .objects
                .into_iter()
                .map(|object| object.family)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let missing_families = spec
        .required_feature_families
        .iter()
        .filter(|family| {
            ObjectTypeName::new(**family)
                .map_or(true, |name| !observed_feature_families.contains(&name))
        })
        .copied()
        .collect::<Vec<_>>();
    let agreement = if observed == spec.expected && missing_families.is_empty() {
        CorpusAgreement::Match
    } else {
        CorpusAgreement::UnexpectedDrift
    };
    let reason = (!matches!(agreement, CorpusAgreement::Match))
        .then(|| {
            if observed == spec.expected {
                format!("missing feature families {}", missing_families.join(","))
            } else {
                format!("expected {:?}, observed {observed:?}", spec.expected)
            }
        })
        .map(|reason| BoundedText::new(reason, 512))
        .transpose()?;
    Ok(CorpusAgreementRow {
        fixture: BoundedText::new(spec.fixture, 512)?,
        source: spec.source,
        vera_pdf_outcome: spec.expected,
        pdfv_outcome: observed,
        profile: BoundedText::new(spec.profile, 128)?,
        agreement,
        semantic_families: bounded_texts(spec.semantic_families, 128)?,
        observed_feature_families: observed_feature_families
            .into_iter()
            .map(|family| BoundedText::new(family.as_str(), 128))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        reason,
    })
}

fn bounded_texts(values: &[&str], max_bytes: usize) -> Result<Vec<BoundedText>> {
    values
        .iter()
        .map(|value| BoundedText::new(*value, max_bytes).map_err(Into::into))
        .collect()
}

fn default_options() -> ValidationOptions {
    ValidationOptions::default()
}

fn feature_options() -> ValidationOptions {
    ValidationOptions::builder()
        .feature_selection(FeatureSelection::All)
        .build()
}

fn explicit_profile_options(
    family: &str,
    part: u32,
    conformance: &str,
) -> Result<ValidationOptions> {
    let flavour = ValidationFlavour::new(
        family,
        std::num::NonZeroU32::new(part).ok_or(crate::ConfigError::InvalidValue {
            field: "part",
            reason: BoundedText::unchecked("profile part must be nonzero"),
        })?,
        conformance,
    )?;
    Ok(ValidationOptions::builder()
        .flavour(FlavourSelection::Explicit { flavour })
        .build())
}

fn minimal_pdf() -> &'static [u8] {
    br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
}

fn content_operator_pdf() -> Vec<u8> {
    let stream =
        b"BT /F1 12 Tf (secret text) Tj ET /Cs1 CS /GS1 gs /Sh1 sh /Im1 Do /Span BMC EMC BI /W 1 /H 1 /BPC 8 /CS /RGB ID x EI WeirdOp";
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> /ColorSpace << /Cs1 /DeviceRGB >> /ExtGState << /GS1 << >> >> /Shading << /Sh1 << >> >> /XObject << /Im1 6 0 R >> >> /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
        .to_vec();
    pdf.extend(stream.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(stream);
    pdf.extend(
        br"
endstream
endobj
5 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
6 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

fn resource_font_color_pdf() -> Vec<u8> {
    let stream = b"BT /Inherited 12 Tf ET /Local cs /Missing gs /Bad Do /Shade sh";
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /OutputIntents [10 0 R] >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /Inherited 5 0 R >> /Shading << /Shade 11 0 R >> >> >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /ColorSpace << /Local 7 0 R >> /XObject << /Bad 6 0 R >> >> /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
        .to_vec();
    pdf.extend(stream.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(stream);
    pdf.extend(
        br"
endstream
endobj
5 0 obj
<< /Type /Font /Subtype /Type0 /BaseFont /Faux /DescendantFonts [8 0 R] /ToUnicode 12 0 R >>
endobj
6 0 obj
<< /NotAnXObject true >>
endobj
7 0 obj
<< /N 3 /Alternate /DeviceRGB /Range [0 1 0 1 0 1] >>
endobj
8 0 obj
<< /Type /Font /Subtype /CIDFontType2 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 9 0 R >>
endobj
9 0 obj
<< /Type /FontDescriptor /FontName /Faux /FontFile2 13 0 R >>
endobj
10 0 obj
<< /Type /OutputIntent /S /GTS_PDFA1 /DestOutputProfile 14 0 R >>
endobj
11 0 obj
<< /ShadingType 2 /ColorSpace /DeviceRGB /Function 15 0 R >>
endobj
12 0 obj
<< /Type /CMap /CMapName /Identity-H /WMode 0 >>
endobj
13 0 obj
<< /Length 4 /Length1 4 >>
stream
font
endstream
endobj
14 0 obj
<< /N 3 /Length 132 >>
stream
",
    );
    let mut icc = vec![0_u8; 132];
    write_fixture_bytes(&mut icc, 0, &132_u32.to_be_bytes());
    write_fixture_byte(&mut icc, 8, 4);
    write_fixture_byte(&mut icc, 9, 0x30);
    write_fixture_bytes(&mut icc, 12, b"mntr");
    write_fixture_bytes(&mut icc, 16, b"RGB ");
    write_fixture_bytes(&mut icc, 20, b"XYZ ");
    write_fixture_bytes(&mut icc, 64, &1_u32.to_be_bytes());
    pdf.extend(icc);
    pdf.extend(
        br"
endstream
endobj
15 0 obj
<< /FunctionType 2 /Domain [0 1] /Range [0 1] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

fn xmp_pdf() -> Vec<u8> {
    let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/">
<pdfaid:part>2</pdfaid:part>
<pdfaid:conformance>B</pdfaid:conformance>
</rdf:Description>
</rdf:RDF>
</x:xmpmeta>"#;
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Metadata 2 0 R >>
endobj
2 0 obj
<< /Type /Metadata /Subtype /XML /Length "
        .to_vec();
    pdf.extend(xmp.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(xmp);
    pdf.extend(
        br"
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

fn accessibility_pdf() -> Vec<u8> {
    let stream = b"/P <</MCID 0>> BDC BT (secret text) Tj ET EMC /Figure <</MCID 1>> BDC /Im1 Do EMC /Artifact BMC EMC";
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 8 0 R /Lang (en-US) /MarkInfo << /Marked true >> >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /StructParents 0 /Resources << /XObject << /Im1 5 0 R >> >> /Annots [6 0 R] /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
        .to_vec();
    pdf.extend(stream.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(stream);
    pdf.extend(
        br"
endstream
endobj
5 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
6 0 obj
<< /Type /Annot /Subtype /Link /StructParent 6 >>
endobj
8 0 obj
<< /Type /StructTreeRoot /K [9 0 R 10 0 R 11 0 R 12 0 R] /RoleMap << /CustomH /H1 >> /ClassMap << /Important << >> >> /IDTree << /Names [(heading) 9 0 R] >> /ParentTree << /Nums [0 [9 0 R 10 0 R] 6 11 0 R] >> /ParentTreeNextKey 7 >>
endobj
9 0 obj
<< /Type /StructElem /S /CustomH /Pg 3 0 R /K 0 /ID (heading) /C /Important >>
endobj
10 0 obj
<< /Type /StructElem /S /Figure /Pg 3 0 R /K << /Type /MCR /Pg 3 0 R /MCID 1 >> /Alt (chart alternative text) >>
endobj
11 0 obj
<< /Type /StructElem /S /Link /Pg 3 0 R /K << /Type /OBJR /Obj 6 0 R /Pg 3 0 R >> >>
endobj
12 0 obj
<< /Type /StructElem /S /L /Pg 3 0 R /K [13 0 R] >>
endobj
13 0 obj
<< /Type /StructElem /S /LI /Pg 3 0 R /K [] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

fn write_fixture_bytes(target: &mut [u8], start: usize, bytes: &[u8]) {
    let end = start.saturating_add(bytes.len());
    if let Some(slot) = target.get_mut(start..end) {
        slot.copy_from_slice(bytes);
    }
}

fn write_fixture_byte(target: &mut [u8], index: usize, byte: u8) {
    if let Some(slot) = target.get_mut(index) {
        *slot = byte;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CorpusAgreement, CorpusOutcome, corpus_agreement_report, explicit_profile_options,
    };

    #[test]
    fn test_should_emit_java_free_semantic_corpus_rows() -> crate::Result<()> {
        let report = corpus_agreement_report()?;

        assert!(!report.live_oracle_enabled);
        assert_eq!(report.summary.total_rows, 9);
        assert_eq!(report.summary.matches, report.summary.total_rows);
        assert_eq!(report.summary.unexpected_drift, 0);
        assert!(report.rows.iter().any(|row| {
            row.fixture.as_str() == "generated:accessibility-structure"
                && row.agreement == CorpusAgreement::Match
                && row
                    .observed_feature_families
                    .iter()
                    .any(|family| family.as_str() == "accessibilityDocument")
        }));
        assert!(report.rows.iter().any(|row| {
            row.fixture.as_str() == "generated:profile-official-pdfa-1b"
                && row.pdfv_outcome == CorpusOutcome::Incomplete
        }));
        Ok(())
    }

    #[test]
    fn test_should_reject_zero_profile_part_in_corpus_options() {
        let result = explicit_profile_options("pdfa", 0, "b");

        assert!(result.is_err());
    }
}
