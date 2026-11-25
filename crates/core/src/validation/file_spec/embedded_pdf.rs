//! Bounded validation of embedded documents against their declared PDF/A profiles.

use std::{collections::HashMap, io::Cursor, sync::Arc};

use super::EMBEDDED_FILE;
use crate::{
    BoundedText, BuiltinProfileRepository, FlavourSelection, ModelValue, ObjectKey, ParseError,
    Parser, PdfvError, ProfileError, ProfileReport, ProfileRepository, PropertyName,
    ResourceLimits, Result, StreamObject, ValidationError, ValidationFlavour, ValidationProfile,
    profile::collect_property_paths,
    validation::{ValidationSession, append_validation_xmp, scoped_pua, unknown_property},
    xmp::parse_document_xmp,
};

pub(super) const PDFA12: &str = "isValidPDFA12";
pub(super) const PDFA124: &str = "isValidPDFA124";
const MAX_EMBEDDING_DEPTH: u32 = 8;
const MAX_EMBEDDED_DOCUMENTS: u64 = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PartLimit {
    Two,
    Four,
}

impl PartLimit {
    fn for_property(property: &str) -> Option<Self> {
        match property {
            PDFA12 => Some(Self::Two),
            PDFA124 => Some(Self::Four),
            _ => None,
        }
    }

    fn allows(self, flavour: &ValidationFlavour) -> bool {
        flavour.family.as_str() == "pdfa"
            && match flavour.part.get() {
                1 | 2 => true,
                4 => self == Self::Four,
                _ => false,
            }
    }
}

#[derive(Clone, Debug)]
enum Verdict {
    Conforming,
    Nonconforming,
    Unverified(BoundedText),
}

impl Verdict {
    fn unverified(reason: &'static str) -> Self {
        Self::Unverified(BoundedText::unchecked(reason))
    }
}

#[derive(Clone, Debug)]
pub(super) struct Conformance {
    pdfa12: Verdict,
    pdfa124: Verdict,
}

impl Conformance {
    fn both(verdict: Verdict) -> Self {
        Self {
            pdfa12: verdict.clone(),
            pdfa124: verdict,
        }
    }

    pub(super) fn property(self, name: &PropertyName) -> Result<ModelValue> {
        let verdict = match PartLimit::for_property(name.as_str()) {
            Some(PartLimit::Two) => self.pdfa12,
            Some(PartLimit::Four) => self.pdfa124,
            None => return unknown_property(name),
        };
        match verdict {
            Verdict::Conforming => Ok(ModelValue::Bool(true)),
            Verdict::Nonconforming => Ok(ModelValue::Bool(false)),
            Verdict::Unverified(reason) => Err(ProfileError::UnsupportedRule { reason }.into()),
        }
    }
}

#[derive(Debug, Default)]
pub(in crate::validation) struct Audit {
    selected: Option<PartLimit>,
    results: HashMap<ObjectKey, Conformance>,
}

impl Audit {
    pub(in crate::validation) fn for_profiles(profiles: &[ValidationProfile]) -> Self {
        let mut selected = None;
        let mut paths = Vec::new();
        for rule in profiles
            .iter()
            .flat_map(|profile| &profile.rules)
            .filter(|rule| rule.object_type.as_str() == EMBEDDED_FILE)
        {
            collect_property_paths(&rule.test, &mut paths);
            selected = selected.max(
                paths
                    .drain(..)
                    .filter_map(|path| PartLimit::for_property(path.parts().first()?.as_str()))
                    .max(),
            );
        }
        Self {
            selected,
            results: HashMap::new(),
        }
    }

    pub(super) fn check(
        &mut self,
        context: &mut Context,
        key: ObjectKey,
        stream: &StreamObject,
        limits: &ResourceLimits,
    ) -> Result<Option<Conformance>> {
        let Some(selected) = self.selected else {
            return Ok(None);
        };
        if let Some(result) = self.results.get(&key) {
            return Ok(Some(result.clone()));
        }
        let result = context.check(stream, limits, selected)?;
        self.results.insert(key, result.clone());
        Ok(Some(result))
    }
}

/// Shared budget for all embedded documents and profiles in a validation.
#[derive(Debug)]
pub(in crate::validation) struct Context {
    depth: u32,
    documents_left: u64,
    input_bytes_left: u64,
    decoded_bytes_left: u64,
    filter_output_left: u64,
    profiles: HashMap<ValidationFlavour, Arc<ValidationProfile>>,
}

impl Context {
    pub(in crate::validation) fn new(limits: &ResourceLimits) -> Self {
        Self {
            depth: 0,
            documents_left: limits.max_objects.min(MAX_EMBEDDED_DOCUMENTS),
            input_bytes_left: limits.max_file_bytes,
            decoded_bytes_left: limits.max_file_bytes,
            filter_output_left: limits.max_file_bytes.saturating_mul(2),
            profiles: HashMap::new(),
        }
    }

    fn check(
        &mut self,
        stream: &StreamObject,
        limits: &ResourceLimits,
        selected: PartLimit,
    ) -> Result<Conformance> {
        if self.depth >= limits.max_object_depth.min(MAX_EMBEDDING_DEPTH) {
            return Ok(Conformance::both(Verdict::unverified(
                "embedded PDF nesting limit reached",
            )));
        }
        let Some(documents_left) = self.documents_left.checked_sub(1) else {
            return Ok(Conformance::both(Verdict::unverified(
                "embedded PDF document count limit reached",
            )));
        };
        let Some(input_bytes_left) = self.input_bytes_left.checked_sub(stream.discovered_length)
        else {
            return Ok(Conformance::both(Verdict::unverified(
                "embedded PDF encoded byte budget exhausted",
            )));
        };
        self.documents_left = documents_left;
        self.input_bytes_left = input_bytes_left;

        self.depth = self.depth.saturating_add(1);
        let result = self.read_document(stream, limits, selected);
        self.depth = self.depth.saturating_sub(1);
        result.or_else(|error| classify_error(error).map(Conformance::both))
    }

    fn read_document(
        &mut self,
        stream: &StreamObject,
        limits: &ResourceLimits,
        selected: PartLimit,
    ) -> Result<Conformance> {
        let mut decode_limits = limits.clone();
        decode_limits.max_stream_decode_bytes =
            limits.max_stream_decode_bytes.min(self.decoded_bytes_left);
        let bytes =
            stream.fully_decoded_bytes(&decode_limits, Some(&mut self.filter_output_left))?;
        self.decoded_bytes_left = self
            .decoded_bytes_left
            .checked_sub(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or(ValidationError::LimitExceeded {
                limit: "embedded PDF decoded bytes",
            })?;
        let mut document = Parser::new(limits.clone()).parse(Cursor::new(bytes))?;
        if document.is_encrypted() {
            return Ok(Conformance::both(Verdict::Nonconforming));
        }
        let xmp = parse_document_xmp(&document, limits, false)?;
        if xmp.decoding_unavailable {
            return Ok(Conformance::both(Verdict::unverified(
                "embedded PDF identification metadata could not be decoded",
            )));
        }
        let Some(flavour) = xmp
            .packet
            .as_ref()
            .and_then(|packet| {
                packet
                    .identification
                    .iter()
                    .find(|claim| selected.allows(&claim.flavour))
            })
            .map(|claim| &claim.flavour)
        else {
            return Ok(Conformance::both(Verdict::Nonconforming));
        };
        let Some(profile) = self.profile(flavour)? else {
            return Ok(Conformance::both(Verdict::Nonconforming));
        };

        let verdict = (|| {
            let profiles = std::slice::from_ref(profile.as_ref());
            append_validation_xmp(&mut document, limits, profiles, xmp)?;
            let mut session = ValidationSession::new(
                document,
                limits.clone(),
                1,
                false,
                Audit::for_profiles(profiles),
            );
            let report = session.validate_profile(
                &profile,
                scoped_pua::ContentMode::for_profiles(profiles),
                self,
            )?;
            Ok(attachment_verdict(report))
        })()
        .or_else(classify_error)?;

        Ok(if profile.flavour.part.get() == 4 {
            Conformance {
                pdfa12: Verdict::Nonconforming,
                pdfa124: verdict,
            }
        } else {
            Conformance::both(verdict)
        })
    }

    fn profile(&mut self, flavour: &ValidationFlavour) -> Result<Option<Arc<ValidationProfile>>> {
        if let Some(profile) = self.profiles.get(flavour) {
            return Ok(Some(Arc::clone(profile)));
        }
        let profiles = BuiltinProfileRepository::new().profiles_for(&FlavourSelection::Explicit {
            flavour: flavour.clone(),
        });
        match profiles {
            Ok(profiles) => {
                let profile = profiles.into_iter().next().map(Arc::new);
                if let Some(profile) = &profile {
                    self.profiles.insert(flavour.clone(), Arc::clone(profile));
                }
                Ok(profile)
            }
            Err(PdfvError::Profile(ProfileError::UnsupportedSelection)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn attachment_verdict(mut report: ProfileReport) -> Verdict {
    if report.is_compliant {
        Verdict::Conforming
    } else if report.failed_rules != 0 {
        Verdict::Nonconforming
    } else {
        report.unsupported_rules.pop().map_or_else(
            || Verdict::unverified("embedded PDF has unsupported required checks"),
            |check| Verdict::Unverified(check.reason),
        )
    }
}

fn classify_error(error: PdfvError) -> Result<Verdict> {
    match error {
        PdfvError::Parse(ParseError::LimitExceeded { limit })
        | PdfvError::Validation(ValidationError::LimitExceeded { limit }) => {
            Ok(Verdict::unverified(limit))
        }
        PdfvError::Parse(ParseError::UnsupportedFilter { .. }) => Ok(Verdict::unverified(
            "embedded PDF stream filter is unsupported",
        )),
        PdfvError::Parse(
            ParseError::Malformed { .. }
            | ParseError::MissingObject { .. }
            | ParseError::StreamDecode { .. }
            | ParseError::ArithmeticOverflow { .. },
        ) => Ok(Verdict::Nonconforming),
        error => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fmt::Write as _, io::Cursor, num::NonZeroU32};

    use rstest::rstest;

    use super::{Audit, Context, MAX_EMBEDDING_DEPTH};
    use crate::{
        CosObject, FlavourSelection, InputName, ObjectKey, ParseError, Parser, ResourceLimits,
        ValidationFlavour, ValidationOptions, ValidationReport, ValidationStatus, Validator,
        import_verapdf_profile_xml,
        validation::{ValidationSession, scoped_pua},
    };

    const RULE_PDFA2: &str = "iso-19005-2-6-8-5";
    const RULE_PDFA4: &str = "iso-19005-4-6-9-3";

    #[rstest]
    #[case(2, 2, "B", true)]
    #[case(2, 2, "U", true)]
    #[case(2, 2, "A", false)]
    #[case(2, 3, "B", false)]
    #[case(2, 4, "", false)]
    #[case(4, 4, "", true)]
    #[case(4, 4, "E", true)]
    fn test_should_validate_the_attached_pdf_using_its_own_profile(
        #[case] parent: u32,
        #[case] child: u32,
        #[case] level: &str,
        #[case] compliant: bool,
    ) -> Result<(), Box<dyn Error>> {
        let rule = if parent == 2 { RULE_PDFA2 } else { RULE_PDFA4 };
        let payload = fixture(child, level, "", None);
        let report = validate(
            parent,
            fixture(parent, parent_level(parent), "", Some((&payload, ""))),
        )?;
        assert_eq!(
            counts(&report, rule)?,
            if compliant { (1, 0, 0) } else { (0, 1, 0) }
        );
        Ok(())
    }

    #[test]
    fn test_should_propagate_invalid_nested_attachment() -> Result<(), Box<dyn Error>> {
        let child = fixture(2, "B", "", Some((b"plain text", "")));
        let report = validate(2, fixture(2, "B", "", Some((&child, ""))))?;
        assert_eq!(counts(&report, RULE_PDFA2)?, (0, 1, 0));
        assert_eq!(report.status, ValidationStatus::Invalid);
        Ok(())
    }

    #[test]
    fn test_should_keep_nesting_limit_as_unverified_evidence() -> Result<(), Box<dyn Error>> {
        let mut bytes = fixture(2, "B", "", None);
        for _ in 0..=MAX_EMBEDDING_DEPTH {
            bytes = fixture(2, "B", "", Some((&bytes, "")));
        }
        let report = validate(2, bytes)?;
        assert_eq!(counts(&report, RULE_PDFA2)?, (0, 0, 1));
        assert_eq!(report.status, ValidationStatus::Incomplete);
        assert!(report.profile_reports.iter().any(|profile| {
            profile
                .unsupported_rules
                .iter()
                .any(|rule| rule.reason.as_str() == "embedded PDF nesting limit reached")
        }));
        Ok(())
    }

    #[test]
    fn test_should_reject_metadata_only_filter_output_as_attachment_evidence()
    -> Result<(), Box<dyn Error>> {
        let filter = "/Filter /DCTDecode";
        let payload = fixture(2, "B", "", None);
        let filtered_metadata = fixture(2, "B", filter, None);
        for (payload, stream_filter) in [(&payload, filter), (&filtered_metadata, "")] {
            let report = validate(2, fixture(2, "B", "", Some((payload, stream_filter))))?;
            assert_eq!(counts(&report, RULE_PDFA2)?, (0, 0, 1));
        }
        Ok(())
    }

    #[test]
    fn test_should_charge_each_decode_stage_and_failed_attempt() -> Result<(), Box<dyn Error>> {
        let limits = ResourceLimits::default();
        let data = fixture(
            2,
            "B",
            "",
            Some((b"3331>", "/Filter [/ASCIIHexDecode /ASCIIHexDecode]")),
        );
        let pdf = Parser::new(limits.clone()).parse(Cursor::new(data))?;
        let key = ObjectKey::new(NonZeroU32::new(7).ok_or("missing stream key")?, 0);
        let stream = pdf
            .objects
            .get(&key)
            .and_then(|object| match &object.object {
                CosObject::Stream(stream) => Some(stream),
                _ => None,
            })
            .ok_or("missing embedded stream")?;
        let mut budget = 3;
        assert_eq!(
            stream.fully_decoded_bytes(&limits, Some(&mut budget))?,
            b"1"
        );
        assert_eq!(budget, 0);

        let mut budget = 1;
        assert!(matches!(
            stream.fully_decoded_bytes(&limits, Some(&mut budget)),
            Err(ParseError::LimitExceeded { .. })
        ));
        assert_eq!(budget, 0);
        Ok(())
    }

    #[test]
    fn test_should_skip_attachment_work_when_the_rule_short_circuits() -> Result<(), Box<dyn Error>>
    {
        let mut bytes = fixture(2, "B", "", Some((b"<record/>", "")));
        let mime = b"application#2fpdf";
        let start = bytes
            .windows(mime.len())
            .position(|window| window == mime)
            .ok_or("missing fixture MIME name")?;
        bytes
            .get_mut(start..start.saturating_add(mime.len()))
            .ok_or("invalid fixture MIME range")?
            .copy_from_slice(b"application#2fxml");
        let profile = import_verapdf_profile_xml(r#"
            <profile xmlns="http://www.verapdf.org/ValidationProfile" flavour="PDFA_2_B">
                <details><name>MIME guard</name><description>Attachment scope</description></details>
                <rules>
                    <rule object="EmbeddedFile">
                        <id specification="ISO_19005_2" clause="6.8" testNumber="99"/>
                        <description>Apply the conformance check to PDF attachments</description>
                        <test>Subtype != "application/pdf" || isValidPDFA12 == true</test>
                        <error><message>Invalid embedded PDF</message></error>
                    </rule>
                </rules>
            </profile>"#)?.profile;
        let limits = ResourceLimits::default();
        let mut context = Context::new(&limits);
        let available = context.documents_left;
        let document = Parser::new(limits.clone()).parse(Cursor::new(bytes))?;
        let profiles = std::slice::from_ref(&profile);
        let mut session =
            ValidationSession::new(document, limits, 1, true, Audit::for_profiles(profiles));
        let report = session.validate_profile(
            &profile,
            scoped_pua::ContentMode::for_profiles(profiles),
            &mut context,
        )?;

        assert!(report.is_compliant);
        assert_eq!(report.checks_executed, 1);
        assert_eq!(context.documents_left, available);
        Ok(())
    }

    fn parent_level(part: u32) -> &'static str {
        if part == 2 { "B" } else { "" }
    }

    fn validate(part: u32, bytes: Vec<u8>) -> Result<ValidationReport, Box<dyn Error>> {
        let part = NonZeroU32::new(part).ok_or("invalid fixture PDF/A part")?;
        let level = if part.get() == 4 { "none" } else { "b" };
        let flavour = FlavourSelection::Explicit {
            flavour: ValidationFlavour::new("pdfa", part, level)?,
        };
        let validator = Validator::new(
            ValidationOptions::builder()
                .flavour(flavour)
                .record_passed_assertions(true)
                .build(),
        )?;
        Ok(validator.validate_reader(Cursor::new(bytes), InputName::memory())?)
    }

    fn counts(
        report: &ValidationReport,
        rule: &str,
    ) -> Result<(usize, usize, usize), Box<dyn Error>> {
        let profile = report
            .profile_reports
            .first()
            .ok_or("missing fixture profile")?;
        let passed = profile
            .passed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule)
            .count();
        let failed = profile
            .failed_assertions
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule)
            .count();
        let unsupported = profile
            .unsupported_rules
            .iter()
            .filter(|item| item.rule_id.0.as_str() == rule)
            .count();
        Ok((passed, failed, unsupported))
    }

    fn fixture(
        part: u32,
        level: &str,
        metadata_filter: &str,
        attachment: Option<(&[u8], &str)>,
    ) -> Vec<u8> {
        let version = if part == 4 { "2.0" } else { "1.7" };
        let mut pdf = format!("%PDF-{version}\n").into_bytes();
        pdf.extend_from_slice(b"%\xe2\xe3\xcf\xd3\n");
        let mut claims = format!("<pdfaid:part>{part}</pdfaid:part>");
        if !level.is_empty() {
            let _ = write!(claims, "<pdfaid:conformance>{level}</pdfaid:conformance>");
        }
        if part == 4 {
            claims.push_str("<pdfaid:rev>2020</pdfaid:rev>");
        }
        let xmp = format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
             <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\
             {claims}</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>"
        );
        let names = if attachment.is_some() {
            "/AF [6 0 R] /Names << /EmbeddedFiles << /Names [(child.pdf) 6 0 R] >> >>"
        } else {
            ""
        };
        let mut bodies = vec![
            format!("<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R {names} >>").into_bytes(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources <<>> /Contents 4 0 R >>".to_vec(),
            stream("", b"q Q"),
            stream(&format!("/Type /Metadata /Subtype /XML {metadata_filter}"), xmp.as_bytes()),
        ];
        if let Some((bytes, filter)) = attachment {
            bodies.push(b"<< /Type /Filespec /F (child.pdf) /UF (child.pdf) /AFRelationship /Data /Desc (Child) /EF << /F 7 0 R /UF 7 0 R >> >>".to_vec());
            bodies.push(stream(
                &format!(
                    "/Type /EmbeddedFile /Subtype /application#2fpdf /Params << /ModDate \
                     (D:20200101000000Z) >> {filter}"
                ),
                bytes,
            ));
        }
        let mut offsets = String::new();
        for (index, body) in bodies.iter().enumerate() {
            let _ = write!(offsets, "{:010} 00000 n\r\n", pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index.saturating_add(1)).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        let size = bodies.len().saturating_add(1);
        pdf.extend_from_slice(
            format!(
                "xref\n0 {size}\n0000000000 65535 f\r\n{offsets}trailer\n<< /Size {size} /Root 1 \
                 0 R /ID [<9d3b4ea31df045438d7f56bb75f15773> <9d3b4ea31df045438d7f56bb75f15773>] \
                 >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    fn stream(attributes: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< {attributes} /Length {} >>\nstream\n", data.len()).into_bytes();
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }
}
