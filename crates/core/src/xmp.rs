//! Bounded XMP packet extraction, identification parsing, and flavour detection.

use std::{collections::BTreeMap, num::NonZeroU32, sync::Arc};

use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};

use crate::{
    BoundedText, CosObject, Identifier, ObjectKey, ParseError, ParseFact, PdfvError,
    ProfileRepository, ResourceLimits, Result, ValidationFlavour, ValidationProfile,
    ValidationWarning, XmpFact, display_flavour,
};

const PDF_A_ID_NS: &str = "http://www.aiim.org/pdfa/ns/id/";
const PDF_UA_ID_NS: &str = "http://www.aiim.org/pdfua/ns/id/";
const PDF_D_NS: &str = "http://pdfa.org/declarations/";
const WTPDF_ACCESSIBILITY_DECLARATION: &str = "http://pdfa.org/declarations/wtpdf#accessibility1.0";
const WTPDF_REUSE_DECLARATION: &str = "http://pdfa.org/declarations/wtpdf#reuse1.0";

/// Namespace declaration retained from an XMP packet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamespaceBinding {
    /// Namespace prefix, or empty for the default namespace.
    pub prefix: Identifier,
    /// Namespace URI.
    pub uri: BoundedText,
}

/// Recognized XMP identification schema kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum XmpIdentificationKind {
    /// PDF/A identification schema.
    PdfA,
    /// PDF/UA identification schema.
    PdfUa,
    /// WTPDF PDF Declaration.
    Wtpdf,
}

/// Recognized flavour claim extracted from XMP metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlavourClaim {
    /// Claim kind.
    pub kind: XmpIdentificationKind,
    /// Validation flavour represented by the claim.
    pub flavour: ValidationFlavour,
    /// Report-safe display spelling.
    pub display_flavour: BoundedText,
    /// Source namespace URI.
    pub namespace_uri: BoundedText,
    /// Source property name.
    pub property: Identifier,
}

/// Parsed XMP packet summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XmpPacket {
    /// Metadata stream object that supplied the packet.
    pub source_object: ObjectKey,
    /// Packet byte count.
    pub bytes: u64,
    /// Retained namespace declarations.
    pub namespaces: Vec<NamespaceBinding>,
    /// Recognized identification claims.
    pub identification: Vec<FlavourClaim>,
    /// Report-safe parser facts.
    pub facts: Vec<XmpFact>,
}

/// Bounded XMP parser.
#[derive(Clone, Debug, Default)]
pub struct XmpParser;

/// Auto flavour detection result.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DetectedFlavours {
    /// Parsed packet, when catalog metadata was present and XML was parseable.
    pub packet: Option<XmpPacket>,
    /// Profiles selected for validation.
    pub profiles: Vec<ValidationProfile>,
    /// Report-safe parse facts generated during detection.
    pub parse_facts: Vec<ParseFact>,
    /// Structured warnings generated during detection.
    pub warnings: Vec<ValidationWarning>,
}

/// Profile selector backed by XMP identification claims.
#[derive(Clone)]
pub struct FlavourDetector {
    profiles: Arc<dyn ProfileRepository + Send + Sync>,
}

impl std::fmt::Debug for FlavourDetector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FlavourDetector")
    }
}

impl FlavourDetector {
    /// Creates a detector backed by a profile repository.
    #[must_use]
    pub fn new(profiles: Arc<dyn ProfileRepository + Send + Sync>) -> Self {
        Self { profiles }
    }

    /// Detects validation profiles from catalog XMP metadata.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when a detected or fallback flavour cannot be loaded.
    pub fn detect(
        &self,
        document: &crate::ParsedDocument,
        default: Option<&ValidationFlavour>,
        limits: &ResourceLimits,
    ) -> Result<DetectedFlavours> {
        let mut warnings = Vec::new();
        let Some((object, bytes)) = catalog_xmp_bytes(document, limits, &mut warnings)? else {
            return self.fallback(default, "catalog metadata stream is missing");
        };
        let parser = XmpParser;
        let packet = match parser.parse_packet(object, &bytes, limits) {
            Ok(packet) => packet,
            Err(PdfvError::Parse(ParseError::LimitExceeded { limit })) => {
                return Err(ParseError::LimitExceeded { limit }.into());
            }
            Err(error) => {
                let reason = BoundedText::new(error.to_string(), 512)
                    .unwrap_or_else(|_| BoundedText::unchecked("XMP parse failed"));
                return self.fallback_with_warning(
                    default,
                    ValidationWarning::AutoDetection { message: reason },
                );
            }
        };

        let mut parse_facts = xmp_parse_facts(&packet)?;
        let mut profiles = Vec::new();
        for claim in &packet.identification {
            match self
                .profiles
                .profiles_for(&crate::FlavourSelection::Explicit {
                    flavour: claim.flavour.clone(),
                }) {
                Ok(mut selected) => profiles.append(&mut selected),
                Err(error) => warnings.push(ValidationWarning::IncompatibleProfile {
                    profile_id: Identifier::new(claim.display_flavour.as_str())?,
                    reason: BoundedText::new(error.to_string(), 512)?,
                }),
            }
        }
        if profiles.is_empty() {
            let mut fallback =
                self.fallback(default, "XMP metadata contains no supported claims")?;
            fallback.parse_facts.append(&mut parse_facts);
            fallback.warnings.extend(warnings);
            fallback.packet = Some(packet);
            return Ok(fallback);
        }
        warnings.extend(packet_warnings(&packet)?);
        Ok(DetectedFlavours {
            packet: Some(packet),
            profiles,
            parse_facts,
            warnings,
        })
    }

    fn fallback(
        &self,
        default: Option<&ValidationFlavour>,
        reason: &'static str,
    ) -> Result<DetectedFlavours> {
        let warning = ValidationWarning::AutoDetection {
            message: BoundedText::unchecked(reason),
        };
        self.fallback_with_warning(default, warning)
    }

    fn fallback_with_warning(
        &self,
        default: Option<&ValidationFlavour>,
        warning: ValidationWarning,
    ) -> Result<DetectedFlavours> {
        let warnings = vec![warning];
        let profiles = if let Some(flavour) = default {
            self.profiles.profiles_for(&crate::FlavourSelection::Auto {
                default: Some(flavour.clone()),
            })?
        } else {
            self.profiles
                .profiles_for(&crate::FlavourSelection::Auto { default: None })?
        };
        Ok(DetectedFlavours {
            packet: None,
            profiles,
            parse_facts: Vec::new(),
            warnings,
        })
    }
}

impl XmpParser {
    /// Parses one catalog metadata stream into report-safe XMP facts.
    ///
    /// # Errors
    ///
    /// Returns [`PdfvError`] when resource limits are exceeded or XML is malformed.
    pub fn parse_packet(
        &self,
        source_object: ObjectKey,
        bytes: &[u8],
        limits: &ResourceLimits,
    ) -> Result<XmpPacket> {
        enforce_xmp_len(bytes.len(), limits.max_xmp_bytes)?;
        let text = std::str::from_utf8(bytes).map_err(|error| crate::ProfileError::InvalidXml {
            reason: BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XMP is not UTF-8")),
        })?;
        let parser = PacketBuilder::new(source_object, bytes.len(), limits);
        parser.parse(text)
    }
}

#[derive(Debug)]
struct PacketBuilder<'a> {
    source_object: ObjectKey,
    byte_len: usize,
    limits: &'a ResourceLimits,
    depth: u32,
    elements: u64,
    namespaces: BTreeMap<String, BoundedText>,
    properties: BTreeMap<(String, String), XmpProperty>,
    stack: Vec<ElementFrame>,
    facts: Vec<XmpFact>,
    saw_packet_wrapper: bool,
}

impl<'a> PacketBuilder<'a> {
    fn new(source_object: ObjectKey, byte_len: usize, limits: &'a ResourceLimits) -> Self {
        Self {
            source_object,
            byte_len,
            limits,
            depth: 0,
            elements: 0,
            namespaces: BTreeMap::new(),
            properties: BTreeMap::new(),
            stack: Vec::with_capacity(usize::try_from(limits.max_xmp_depth).unwrap_or(0)),
            facts: Vec::new(),
            saw_packet_wrapper: false,
        }
    }

    fn parse(mut self, text: &str) -> Result<XmpPacket> {
        let mut reader = Reader::from_str(text);
        reader.config_mut().trim_text(true);
        loop {
            match reader.read_event().map_err(|error| xmp_xml_error(&error))? {
                Event::Start(element) => self.start(&element)?,
                Event::Empty(element) => {
                    self.start(&element)?;
                    self.end()?;
                }
                Event::Text(text) => {
                    let decoded =
                        text.decode()
                            .map_err(|error| crate::ProfileError::InvalidXml {
                                reason: bounded_reason(error.to_string()),
                            })?;
                    self.text(decoded.as_ref())?;
                }
                Event::End(_) => self.end()?,
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::CData(_) => {}
                Event::DocType(_) | Event::GeneralRef(_) => {
                    return Err(crate::ProfileError::InvalidXml {
                        reason: BoundedText::unchecked(
                            "XMP DTD and entity processing are forbidden",
                        ),
                    }
                    .into());
                }
                Event::Eof => break,
            }
        }
        self.finish()
    }

    fn start(&mut self, element: &quick_xml::events::BytesStart<'_>) -> Result<()> {
        self.depth = self.depth.checked_add(1).ok_or(ParseError::LimitExceeded {
            limit: "max_xmp_depth",
        })?;
        if self.depth > self.limits.max_xmp_depth {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_depth",
            }
            .into());
        }
        self.elements = self
            .elements
            .checked_add(1)
            .ok_or(ParseError::LimitExceeded {
                limit: "max_xmp_elements",
            })?;
        if self.elements > self.limits.max_xmp_elements {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_elements",
            }
            .into());
        }
        let (prefix, local) = split_xml_name(element.name().as_ref())?;
        if local.as_str() == "xmpmeta" || local.as_str() == "RDF" {
            self.saw_packet_wrapper = true;
        }
        self.read_namespaces(element)?;
        let namespace = self.resolve_prefix(&prefix)?;
        let frame = ElementFrame {
            namespace,
            local,
            text: String::new(),
        };
        self.capture_attr_properties(element)?;
        self.stack.push(frame);
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        let Some(frame) = self.stack.pop() else {
            return Err(crate::ProfileError::InvalidXml {
                reason: BoundedText::unchecked("XMP element depth underflow"),
            }
            .into());
        };
        let value = frame.text.trim().to_owned();
        if !value.is_empty() && is_identification_property(&frame.namespace, &frame.local) {
            self.insert_property(frame, &value)?;
        }
        self.depth = self.depth.checked_sub(1).ok_or(ParseError::LimitExceeded {
            limit: "max_xmp_depth",
        })?;
        Ok(())
    }

    fn text(&mut self, value: &str) -> Result<()> {
        let Some(frame) = self.stack.last_mut() else {
            return Ok(());
        };
        let next_len =
            frame
                .text
                .len()
                .checked_add(value.len())
                .ok_or(ParseError::LimitExceeded {
                    limit: "max_xmp_text_bytes",
                })?;
        if next_len > self.limits.max_xmp_text_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_text_bytes",
            }
            .into());
        }
        frame.text.push_str(value);
        Ok(())
    }

    fn finish(mut self) -> Result<XmpPacket> {
        if !self.saw_packet_wrapper {
            self.facts.push(XmpFact::MissingPacketWrapper);
        }
        let identification = self.claims()?;
        self.facts.push(XmpFact::PacketParsed {
            bytes: checked_u64_len(self.byte_len, "XMP packet length")?,
            namespaces: checked_u64_len(self.namespaces.len(), "XMP namespace count")?,
            claims: checked_u64_len(identification.len(), "XMP claim count")?,
        });
        for claim in &identification {
            self.facts.push(XmpFact::FlavourClaim {
                family: claim.flavour.family.clone(),
                display_flavour: claim.display_flavour.clone(),
                namespace_uri: claim.namespace_uri.clone(),
            });
        }
        let namespaces = self
            .namespaces
            .into_iter()
            .map(|(prefix, uri)| {
                Ok(NamespaceBinding {
                    prefix: Identifier::new(prefix)?,
                    uri,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(XmpPacket {
            source_object: self.source_object,
            bytes: checked_u64_len(self.byte_len, "XMP packet length")?,
            namespaces,
            identification,
            facts: self.facts,
        })
    }

    fn read_namespaces(&mut self, element: &quick_xml::events::BytesStart<'_>) -> Result<()> {
        let mut attributes = 0_usize;
        for attr in element.attributes().with_checks(true) {
            let attr = attr.map_err(|error| crate::ProfileError::InvalidXml {
                reason: bounded_reason(error.to_string()),
            })?;
            attributes = attributes.checked_add(1).ok_or(ParseError::LimitExceeded {
                limit: "max_xmp_attributes",
            })?;
            if attributes > self.limits.max_xmp_attributes {
                return Err(ParseError::LimitExceeded {
                    limit: "max_xmp_attributes",
                }
                .into());
            }
            let key = attr.key.as_ref();
            let prefix = namespace_decl_prefix(key);
            if let Some(prefix) = prefix {
                if self.namespaces.len() >= self.limits.max_xmp_namespaces {
                    return Err(ParseError::LimitExceeded {
                        limit: "max_xmp_namespaces",
                    }
                    .into());
                }
                let value = String::from_utf8_lossy(attr.value.as_ref()).into_owned();
                self.namespaces
                    .insert(prefix, BoundedText::new(value, 512)?);
            }
        }
        Ok(())
    }

    fn capture_attr_properties(
        &mut self,
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<()> {
        for attr in element.attributes().with_checks(true) {
            let attr = attr.map_err(|error| crate::ProfileError::InvalidXml {
                reason: bounded_reason(error.to_string()),
            })?;
            let key = attr.key.as_ref();
            if namespace_decl_prefix(key).is_some() {
                continue;
            }
            let (prefix, local) = split_xml_name(key)?;
            let namespace = self.resolve_prefix(&prefix)?;
            if is_identification_property(&namespace, &local) {
                let value = String::from_utf8_lossy(attr.value.as_ref()).into_owned();
                let frame = ElementFrame {
                    namespace,
                    local,
                    text: String::new(),
                };
                self.insert_property(frame, value.trim())?;
            }
        }
        Ok(())
    }

    fn insert_property(&mut self, frame: ElementFrame, value: &str) -> Result<()> {
        let text = BoundedText::new(value.to_owned(), self.limits.max_xmp_text_bytes)?;
        self.properties.insert(
            (
                frame.namespace.as_str().to_owned(),
                frame.local.as_str().to_owned(),
            ),
            XmpProperty {
                namespace: frame.namespace,
                local: frame.local,
                value: text,
            },
        );
        Ok(())
    }

    fn resolve_prefix(&self, prefix: &Identifier) -> Result<BoundedText> {
        if prefix.as_str().is_empty() {
            return Ok(self
                .namespaces
                .get("")
                .cloned()
                .unwrap_or_else(|| BoundedText::unchecked("")));
        }
        self.namespaces
            .get(prefix.as_str())
            .cloned()
            .ok_or_else(|| {
                crate::ProfileError::InvalidXml {
                    reason: BoundedText::new(
                        format!("unknown XMP namespace prefix {}", prefix.as_str()),
                        512,
                    )
                    .unwrap_or_else(|_| BoundedText::unchecked("unknown XMP namespace prefix")),
                }
                .into()
            })
    }

    fn property(&self, namespace: &str, local: &str) -> Option<&XmpProperty> {
        self.properties
            .get(&(namespace.to_owned(), local.to_owned()))
    }

    fn claims(&self) -> Result<Vec<FlavourClaim>> {
        let mut claims = Vec::new();
        if let Some(claim) = self.pdfa_claim()? {
            claims.push(claim);
        }
        if let Some(claim) = self.pdfua_claim()? {
            claims.push(claim);
        }
        claims.extend(self.wtpdf_claims()?);
        Ok(claims)
    }

    fn pdfa_claim(&self) -> Result<Option<FlavourClaim>> {
        let Some(part) = self.property(PDF_A_ID_NS, "part") else {
            return Ok(None);
        };
        let part_number = parse_nonzero_part(part.value.as_str(), "PDF/A part")?;
        let conformance = self
            .property(PDF_A_ID_NS, "conformance")
            .map_or("none", |property| property.value.as_str())
            .to_ascii_lowercase();
        let flavour = ValidationFlavour::new("pdfa", part_number, conformance)?;
        Ok(Some(claim_from_property(
            XmpIdentificationKind::PdfA,
            &flavour,
            part,
        )?))
    }

    fn pdfua_claim(&self) -> Result<Option<FlavourClaim>> {
        let Some(part) = self.property(PDF_UA_ID_NS, "part") else {
            return Ok(None);
        };
        let part_number = parse_nonzero_part(part.value.as_str(), "PDF/UA part")?;
        let conformance = if part_number.get() == 2 {
            "iso32005"
        } else {
            "none"
        };
        let flavour = ValidationFlavour::new("pdfua", part_number, conformance)?;
        Ok(Some(claim_from_property(
            XmpIdentificationKind::PdfUa,
            &flavour,
            part,
        )?))
    }

    fn wtpdf_claims(&self) -> Result<Vec<FlavourClaim>> {
        let mut claims = Vec::new();
        for property in self
            .properties
            .values()
            .filter(|property| property.namespace.as_str() == PDF_D_NS)
        {
            let conformance = match property.value.as_str() {
                WTPDF_ACCESSIBILITY_DECLARATION => Some("accessibility"),
                WTPDF_REUSE_DECLARATION => Some("reuse"),
                _ => None,
            };
            if let Some(conformance) = conformance {
                let flavour = ValidationFlavour::new("wtpdf", NonZeroU32::MIN, conformance)?;
                claims.push(claim_from_property(
                    XmpIdentificationKind::Wtpdf,
                    &flavour,
                    property,
                )?);
            }
        }
        Ok(claims)
    }
}

#[derive(Clone, Debug)]
struct ElementFrame {
    namespace: BoundedText,
    local: Identifier,
    text: String,
}

#[derive(Clone, Debug)]
struct XmpProperty {
    namespace: BoundedText,
    local: Identifier,
    value: BoundedText,
}

fn catalog_xmp_bytes(
    document: &crate::ParsedDocument,
    limits: &ResourceLimits,
    warnings: &mut Vec<ValidationWarning>,
) -> Result<Option<(ObjectKey, Vec<u8>)>> {
    let Some(catalog_key) = document.catalog else {
        return Ok(None);
    };
    let Some(catalog) = document.objects.get(&catalog_key) else {
        return Ok(None);
    };
    let Some(dictionary) = catalog.object.as_dictionary() else {
        return Ok(None);
    };
    let Some(CosObject::Reference(metadata_key)) = dictionary.get("Metadata") else {
        return Ok(None);
    };
    let Some(metadata) = document.objects.get(metadata_key) else {
        return Ok(None);
    };
    let CosObject::Stream(stream) = &metadata.object else {
        warnings.push(ValidationWarning::AutoDetection {
            message: BoundedText::unchecked("catalog metadata is not a stream"),
        });
        return Ok(None);
    };
    let bytes = stream.decoded_bytes(limits)?;
    enforce_xmp_len(bytes.len(), limits.max_xmp_bytes)?;
    Ok(Some((*metadata_key, bytes)))
}

fn xmp_parse_facts(packet: &XmpPacket) -> Result<Vec<ParseFact>> {
    packet
        .facts
        .iter()
        .cloned()
        .map(|fact| {
            Ok(ParseFact::Xmp {
                object: packet.source_object,
                fact,
            })
        })
        .collect()
}

fn packet_warnings(packet: &XmpPacket) -> Result<Vec<ValidationWarning>> {
    packet
        .facts
        .iter()
        .filter_map(|fact| match fact {
            XmpFact::MissingPacketWrapper => Some("XMP packet wrapper is missing"),
            XmpFact::Malformed { .. } | XmpFact::HostileXmlRejected { .. } => {
                Some("XMP metadata has parser warnings")
            }
            XmpFact::PacketParsed { .. } | XmpFact::FlavourClaim { .. } => None,
        })
        .map(|message| {
            Ok(ValidationWarning::AutoDetection {
                message: BoundedText::new(message, 256)?,
            })
        })
        .collect()
}

fn claim_from_property(
    kind: XmpIdentificationKind,
    flavour: &ValidationFlavour,
    property: &XmpProperty,
) -> Result<FlavourClaim> {
    Ok(FlavourClaim {
        kind,
        flavour: flavour.clone(),
        display_flavour: display_flavour(flavour)?,
        namespace_uri: property.namespace.clone(),
        property: property.local.clone(),
    })
}

fn namespace_decl_prefix(key: &[u8]) -> Option<String> {
    if key == b"xmlns" {
        return Some(String::new());
    }
    key.strip_prefix(b"xmlns:")
        .map(|prefix| String::from_utf8_lossy(prefix).into_owned())
}

fn split_xml_name(name: &[u8]) -> Result<(Identifier, Identifier)> {
    let split = name.iter().position(|byte| *byte == b':');
    let (prefix, local) = match split {
        Some(index) => {
            let prefix = name.get(..index).unwrap_or_default();
            let local = name.get(index.saturating_add(1)..).unwrap_or_default();
            (prefix, local)
        }
        None => (&[][..], name),
    };
    Ok((
        Identifier::new(String::from_utf8_lossy(prefix).into_owned())?,
        Identifier::new(String::from_utf8_lossy(local).into_owned())?,
    ))
}

fn is_identification_property(namespace: &BoundedText, local: &Identifier) -> bool {
    matches!(
        (namespace.as_str(), local.as_str()),
        (PDF_A_ID_NS, "part" | "conformance" | "rev")
            | (PDF_UA_ID_NS, "part" | "rev" | "amd" | "corr")
            | (PDF_D_NS, "conformsTo" | "declarations" | "value" | "li")
    )
}

fn parse_nonzero_part(value: &str, field: &'static str) -> Result<NonZeroU32> {
    let number = value
        .parse::<u32>()
        .map_err(|_| crate::ProfileError::InvalidField {
            field,
            reason: BoundedText::unchecked("XMP identification part is not numeric"),
        })?;
    NonZeroU32::new(number)
        .ok_or(crate::ProfileError::InvalidField {
            field,
            reason: BoundedText::unchecked("XMP identification part is zero"),
        })
        .map_err(Into::into)
}

fn enforce_xmp_len(len: usize, max: u64) -> Result<()> {
    if checked_u64_len(len, "XMP byte length")? > max {
        return Err(ParseError::LimitExceeded {
            limit: "max_xmp_bytes",
        }
        .into());
    }
    Ok(())
}

fn checked_u64_len(len: usize, context: &'static str) -> Result<u64> {
    u64::try_from(len)
        .map_err(|_| ParseError::ArithmeticOverflow { context })
        .map_err(Into::into)
}

fn xmp_xml_error(error: &quick_xml::Error) -> PdfvError {
    crate::ProfileError::InvalidXml {
        reason: bounded_reason(error.to_string()),
    }
    .into()
}

fn bounded_reason(value: String) -> BoundedText {
    BoundedText::new(value, 512).unwrap_or_else(|_| BoundedText::unchecked("XMP XML error"))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::XmpParser;
    use crate::{ObjectKey, ResourceLimits, XmpFact};

    fn key() -> ObjectKey {
        ObjectKey::new(NonZeroU32::MIN, 0)
    }

    #[test]
    fn test_should_parse_pdfa_claim_with_namespace_alias() -> crate::Result<()> {
        let xml = br#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:aid="http://www.aiim.org/pdfa/ns/id/" aid:part="2" aid:conformance="B"/>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert_eq!(packet.identification.len(), 1);
        assert_eq!(
            packet
                .identification
                .first()
                .map(|claim| claim.display_flavour.as_str()),
            Some("pdfa-2b")
        );
        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::FlavourClaim {
                family,
                display_flavour,
                ..
            } if family.as_str() == "pdfa" && display_flavour.as_str() == "pdfa-2b"
        )));
        Ok(())
    }

    #[test]
    fn test_should_parse_pdfua_and_wtpdf_claims() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfuaid="http://www.aiim.org/pdfua/ns/id/"
                     xmlns:pdfd="http://pdfa.org/declarations/"
                     pdfuaid:part="2">
      <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#reuse1.0</pdfd:conformsTo>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;
        let flavours = packet
            .identification
            .iter()
            .map(|claim| claim.display_flavour.as_str())
            .collect::<Vec<_>>();

        assert!(flavours.contains(&"pdfua-2-iso32005"));
        assert!(flavours.contains(&"wtpdf-1-0-reuse"));
        Ok(())
    }

    #[test]
    fn test_should_reject_xmp_doctype_and_entities() {
        let xml = br#"<!DOCTYPE x [ <!ENTITY ext SYSTEM "file:///etc/passwd"> ]>
<x:xmpmeta xmlns:x="adobe:ns:meta/">&ext;</x:xmpmeta>"#;

        let result = XmpParser.parse_packet(key(), xml, &ResourceLimits::default());

        assert!(result.is_err());
    }

    #[test]
    fn test_should_enforce_xmp_byte_cap() {
        let limits = ResourceLimits {
            max_xmp_bytes: 8,
            ..ResourceLimits::default()
        };

        let result = XmpParser.parse_packet(key(), b"<x:xmpmeta/>", &limits);

        assert!(matches!(
            result,
            Err(crate::PdfvError::Parse(crate::ParseError::LimitExceeded {
                limit: "max_xmp_bytes"
            }))
        ));
    }
}
