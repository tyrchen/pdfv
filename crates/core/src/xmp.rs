//! Bounded XMP packet extraction, identification parsing, and flavour detection.

use std::{
    char,
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::Arc,
};

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
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
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
    /// Claimed identification part.
    pub part: NonZeroU32,
    /// Claimed conformance level when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conformance: Option<Identifier>,
    /// Namespace prefix used by the part property.
    pub part_prefix: Identifier,
    /// Report-safe display spelling.
    pub display_flavour: BoundedText,
    /// Source namespace URI.
    pub namespace_uri: BoundedText,
    /// Source property name.
    pub property: Identifier,
    /// Claimed revision value when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<BoundedText>,
    /// Namespace prefix used by the revision property when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev_prefix: Option<Identifier>,
    /// Namespace prefix used by the conformance property when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conformance_prefix: Option<Identifier>,
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

/// Report-safe XMP parse result independent of profile selection.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub(crate) struct XmpParseResult {
    /// Parsed packet, when XML was parseable.
    pub packet: Option<XmpPacket>,
    /// Report-safe parse facts generated during parsing.
    pub parse_facts: Vec<ParseFact>,
    /// Structured warnings generated during parsing.
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
        let parsed_xmp = parse_document_xmp(document, limits, true)?;
        let Some(packet) = parsed_xmp.packet else {
            let mut fallback = self.fallback(default, "catalog metadata stream is missing")?;
            fallback.parse_facts = parsed_xmp.parse_facts;
            fallback.warnings = parsed_xmp.warnings;
            return Ok(fallback);
        };

        let mut parse_facts = parsed_xmp.parse_facts;
        let mut warnings = parsed_xmp.warnings;
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
        let compatible_profiles = select_compatible_profiles(profiles, &mut warnings)?;
        Ok(DetectedFlavours {
            packet: Some(packet),
            profiles: compatible_profiles,
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
        let (text, actual_encoding) = decode_xmp_text(bytes)?;
        let parser = PacketBuilder::new(source_object, bytes.len(), actual_encoding, limits);
        parser.parse(&text)
    }
}

fn decode_xmp_text(bytes: &[u8]) -> Result<(String, Identifier)> {
    if let Some(body) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text = std::str::from_utf8(body).map_err(|error| crate::ProfileError::InvalidXml {
            reason: bounded_reason(error.to_string()),
        })?;
        return Ok((text.to_owned(), Identifier::unchecked("UTF-8")));
    }
    if let Some(body) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16_text(body, Utf16ByteOrder::BigEndian)
            .map(|text| (text, Identifier::unchecked("UTF-16BE")));
    }
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]) {
        return decode_utf32_text(body, Utf32ByteOrder::LittleEndian)
            .map(|text| (text, Identifier::unchecked("UTF-32LE")));
    }
    if let Some(body) = bytes.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]) {
        return decode_utf32_text(body, Utf32ByteOrder::BigEndian)
            .map(|text| (text, Identifier::unchecked("UTF-32BE")));
    }
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16_text(body, Utf16ByteOrder::LittleEndian)
            .map(|text| (text, Identifier::unchecked("UTF-16LE")));
    }
    let text = std::str::from_utf8(bytes).map_err(|error| crate::ProfileError::InvalidXml {
        reason: BoundedText::new(error.to_string(), 512)
            .unwrap_or_else(|_| BoundedText::unchecked("XMP is not UTF-8")),
    })?;
    Ok((text.to_owned(), Identifier::unchecked("UTF-8")))
}

#[derive(Clone, Copy, Debug)]
enum Utf16ByteOrder {
    BigEndian,
    LittleEndian,
}

fn decode_utf16_text(bytes: &[u8], byte_order: Utf16ByteOrder) -> Result<String> {
    let mut chunks = bytes.chunks_exact(2);
    let units = chunks
        .by_ref()
        .map(|chunk| {
            let [first, second] = chunk else {
                return Err(crate::ProfileError::InvalidXml {
                    reason: BoundedText::unchecked("UTF-16 XMP has incomplete code unit"),
                }
                .into());
            };
            let unit = match byte_order {
                Utf16ByteOrder::BigEndian => u16::from_be_bytes([*first, *second]),
                Utf16ByteOrder::LittleEndian => u16::from_le_bytes([*first, *second]),
            };
            Ok(unit)
        })
        .collect::<Result<Vec<_>>>()?;
    if !chunks.remainder().is_empty() {
        return Err(crate::ProfileError::InvalidXml {
            reason: BoundedText::unchecked("UTF-16 XMP has incomplete code unit"),
        }
        .into());
    }
    char::decode_utf16(units)
        .map(|decoded| {
            decoded.map_err(|error| crate::ProfileError::InvalidXml {
                reason: bounded_reason(error.to_string()),
            })
        })
        .collect::<std::result::Result<String, _>>()
        .map_err(Into::into)
}

#[derive(Clone, Copy, Debug)]
enum Utf32ByteOrder {
    BigEndian,
    LittleEndian,
}

fn decode_utf32_text(bytes: &[u8], byte_order: Utf32ByteOrder) -> Result<String> {
    let mut chunks = bytes.chunks_exact(4);
    let mut text = String::new();
    for chunk in chunks.by_ref() {
        let [first, second, third, fourth] = chunk else {
            return Err(crate::ProfileError::InvalidXml {
                reason: BoundedText::unchecked("UTF-32 XMP has incomplete code point"),
            }
            .into());
        };
        let value = match byte_order {
            Utf32ByteOrder::BigEndian => u32::from_be_bytes([*first, *second, *third, *fourth]),
            Utf32ByteOrder::LittleEndian => u32::from_le_bytes([*first, *second, *third, *fourth]),
        };
        let Some(character) = char::from_u32(value) else {
            return Err(crate::ProfileError::InvalidXml {
                reason: BoundedText::unchecked("UTF-32 XMP has invalid code point"),
            }
            .into());
        };
        text.push(character);
    }
    if !chunks.remainder().is_empty() {
        return Err(crate::ProfileError::InvalidXml {
            reason: BoundedText::unchecked("UTF-32 XMP has incomplete code point"),
        }
        .into());
    }
    Ok(text)
}

#[derive(Debug)]
struct PacketBuilder<'a> {
    source_object: ObjectKey,
    byte_len: usize,
    actual_encoding: Identifier,
    limits: &'a ResourceLimits,
    depth: u32,
    elements: u64,
    processing_instructions: u64,
    namespaces: BTreeMap<String, BoundedText>,
    current_namespaces: BTreeMap<String, BoundedText>,
    properties: Vec<XmpProperty>,
    stack: Vec<ElementFrame>,
    facts: Vec<XmpFact>,
    saw_packet_wrapper: bool,
    packet_bytes: Option<BoundedText>,
    packet_encoding: Option<BoundedText>,
}

impl<'a> PacketBuilder<'a> {
    fn new(
        source_object: ObjectKey,
        byte_len: usize,
        actual_encoding: Identifier,
        limits: &'a ResourceLimits,
    ) -> Self {
        Self {
            source_object,
            byte_len,
            actual_encoding,
            limits,
            depth: 0,
            elements: 0,
            processing_instructions: 0,
            namespaces: BTreeMap::new(),
            current_namespaces: BTreeMap::new(),
            properties: Vec::new(),
            stack: Vec::with_capacity(usize::try_from(limits.max_xmp_depth).unwrap_or(0)),
            facts: Vec::new(),
            saw_packet_wrapper: false,
            packet_bytes: None,
            packet_encoding: None,
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
                Event::PI(instruction) => self.processing_instruction(&instruction)?,
                Event::CData(text) => {
                    let decoded =
                        text.decode()
                            .map_err(|error| crate::ProfileError::InvalidXml {
                                reason: bounded_reason(error.to_string()),
                            })?;
                    self.text(decoded.as_ref())?;
                }
                Event::Decl(_) | Event::Comment(_) => {}
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
        let previous_namespaces = self.current_namespaces.clone();
        self.read_namespaces(element)?;
        let namespace = self.resolve_prefix(&prefix)?;
        let in_pdfd_declarations = is_pdfd_declarations_context(&self.stack, &namespace, &local);
        let frame = ElementFrame {
            namespace,
            prefix,
            local,
            array_kind: None,
            xml_lang: self.xml_lang(element)?,
            text: String::new(),
            previous_namespaces,
            in_pdfd_declarations,
        };
        self.capture_attr_properties(element)?;
        if frame.namespace.as_str() == RDF_NS
            && matches!(frame.local.as_str(), "Alt" | "Bag" | "Seq")
            && let Some(property) = self.stack.last()
            && is_rdf_property(&property.namespace, &property.local)
        {
            let container = ElementFrame {
                namespace: property.namespace.clone(),
                prefix: property.prefix.clone(),
                local: property.local.clone(),
                array_kind: Some(frame.local.clone()),
                xml_lang: None,
                text: String::new(),
                previous_namespaces: BTreeMap::new(),
                in_pdfd_declarations: property.in_pdfd_declarations,
            };
            self.insert_property(&container, "")?;
        }
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
        if !value.is_empty() {
            if let Some(array_property) = self.array_item_property(&frame) {
                self.insert_property(&array_property, &value)?;
            } else if is_rdf_property(&frame.namespace, &frame.local) {
                self.insert_property(&frame, &value)?;
            }
        }
        self.current_namespaces = frame.previous_namespaces;
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
        self.facts.push(XmpFact::PacketHeader {
            bytes: self.packet_bytes.clone(),
            encoding: self.packet_encoding.clone(),
            actual_encoding: self.actual_encoding.clone(),
        });
        self.facts.push(XmpFact::PacketParsed {
            bytes: checked_u64_len(self.byte_len, "XMP packet length")?,
            namespaces: checked_u64_len(self.namespaces.len(), "XMP namespace count")?,
            claims: checked_u64_len(identification.len(), "XMP claim count")?,
        });
        for claim in &identification {
            self.facts.push(XmpFact::FlavourClaim {
                family: claim.flavour.family.clone(),
                part: claim.part.get(),
                conformance: claim.conformance.clone(),
                part_prefix: claim.part_prefix.clone(),
                display_flavour: claim.display_flavour.clone(),
                namespace_uri: claim.namespace_uri.clone(),
                rev: claim.rev.clone(),
                rev_prefix: claim.rev_prefix.clone(),
                conformance_prefix: claim.conformance_prefix.clone(),
            });
        }
        for property in &self.properties {
            self.facts.push(XmpFact::RdfProperty {
                namespace_uri: property.namespace.clone(),
                prefix: property.prefix.clone(),
                name: property.local.clone(),
                value: Some(property.value.clone()),
                array_kind: property.array_kind.clone(),
                xml_lang: property.xml_lang.clone(),
            });
        }
        let namespaces = self
            .namespaces
            .into_iter()
            .map(|(prefix, uri)| {
                Ok(NamespaceBinding {
                    prefix: identifier_allow_empty(prefix)?,
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
                let value = BoundedText::new(value, 512)?;
                self.current_namespaces
                    .insert(prefix.clone(), value.clone());
                self.namespaces.insert(prefix, value);
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
            if prefix.as_str() == "xml" {
                continue;
            }
            let namespace = self.resolve_prefix(&prefix)?;
            if is_rdf_property(&namespace, &local) {
                let value = String::from_utf8_lossy(attr.value.as_ref()).into_owned();
                let frame = ElementFrame {
                    namespace,
                    prefix,
                    local,
                    array_kind: None,
                    xml_lang: None,
                    text: String::new(),
                    previous_namespaces: BTreeMap::new(),
                    in_pdfd_declarations: self
                        .stack
                        .last()
                        .is_some_and(|frame| frame.in_pdfd_declarations),
                };
                self.insert_property(&frame, value.trim())?;
            }
        }
        Ok(())
    }

    fn insert_property(&mut self, frame: &ElementFrame, value: &str) -> Result<()> {
        if self.properties.len() >= self.limits.max_xmp_properties {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_properties",
            }
            .into());
        }
        let text = BoundedText::new(value.to_owned(), self.limits.max_xmp_text_bytes)?;
        if is_identification_property(&frame.namespace, &frame.local)
            && self.properties.iter().any(|property| {
                property.namespace == frame.namespace && property.local == frame.local
            })
        {
            self.facts.push(XmpFact::DuplicateClaim {
                namespace_uri: frame.namespace.clone(),
                property: frame.local.clone(),
            });
        }
        self.properties.push(XmpProperty {
            namespace: frame.namespace.clone(),
            prefix: frame.prefix.clone(),
            local: frame.local.clone(),
            value: text,
            array_kind: frame.array_kind.clone(),
            xml_lang: frame.xml_lang.clone(),
            in_pdfd_declarations: frame.in_pdfd_declarations,
        });
        Ok(())
    }

    fn processing_instruction(
        &mut self,
        instruction: &quick_xml::events::BytesPI<'_>,
    ) -> Result<()> {
        self.processing_instructions =
            self.processing_instructions
                .checked_add(1)
                .ok_or(ParseError::LimitExceeded {
                    limit: "max_xmp_processing_instructions",
                })?;
        if self.processing_instructions > self.limits.max_xmp_processing_instructions {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_processing_instructions",
            }
            .into());
        }
        if instruction.content().len() > self.limits.max_xmp_text_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_xmp_text_bytes",
            }
            .into());
        }
        if instruction.target() == b"xpacket" {
            self.saw_packet_wrapper = true;
            let content = String::from_utf8_lossy(instruction.content());
            self.packet_bytes = xpacket_attr(&content, "bytes", self.limits.max_xmp_text_bytes)?;
            self.packet_encoding =
                xpacket_attr(&content, "encoding", self.limits.max_xmp_text_bytes)?;
        }
        Ok(())
    }

    fn xml_lang(&self, element: &quick_xml::events::BytesStart<'_>) -> Result<Option<BoundedText>> {
        for attr in element.attributes().with_checks(true) {
            let attr = attr.map_err(|error| crate::ProfileError::InvalidXml {
                reason: bounded_reason(error.to_string()),
            })?;
            if namespace_decl_prefix(attr.key.as_ref()).is_some() {
                continue;
            }
            let (prefix, local) = split_xml_name(attr.key.as_ref())?;
            let is_xml_namespace = prefix.as_str() == "xml"
                || (!prefix.as_str().is_empty()
                    && self.resolve_prefix(&prefix)?.as_str() == XML_NS);
            if is_xml_namespace && local.as_str() == "lang" {
                let value = String::from_utf8_lossy(attr.value.as_ref()).into_owned();
                return Ok(Some(BoundedText::new(
                    value,
                    self.limits.max_xmp_text_bytes,
                )?));
            }
        }
        Ok(None)
    }

    fn array_item_property(&self, frame: &ElementFrame) -> Option<ElementFrame> {
        if frame.namespace.as_str() != RDF_NS || frame.local.as_str() != "li" {
            return None;
        }
        let array = self.stack.last()?;
        if array.namespace.as_str() != RDF_NS
            || !matches!(array.local.as_str(), "Alt" | "Bag" | "Seq")
        {
            return None;
        }
        let property = self.stack.iter().rev().nth(1)?;
        if !is_rdf_property(&property.namespace, &property.local) {
            return None;
        }
        Some(ElementFrame {
            namespace: property.namespace.clone(),
            prefix: property.prefix.clone(),
            local: property.local.clone(),
            array_kind: Some(array.local.clone()),
            xml_lang: frame.xml_lang.clone(),
            text: String::new(),
            previous_namespaces: BTreeMap::new(),
            in_pdfd_declarations: property.in_pdfd_declarations,
        })
    }

    fn resolve_prefix(&self, prefix: &Identifier) -> Result<BoundedText> {
        if prefix.as_str().is_empty() {
            return Ok(self
                .current_namespaces
                .get("")
                .cloned()
                .unwrap_or_else(|| BoundedText::unchecked("")));
        }
        self.current_namespaces
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
        self.properties.iter().find(|property| {
            property.namespace.as_str() == namespace && property.local.as_str() == local
        })
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
        let mut seen = BTreeSet::new();
        claims.retain(|claim| seen.insert(claim.display_flavour.as_str().to_owned()));
        Ok(claims)
    }

    fn pdfa_claim(&self) -> Result<Option<FlavourClaim>> {
        let Some(part) = self.property(PDF_A_ID_NS, "part") else {
            return Ok(None);
        };
        let part_number = parse_nonzero_part(part.value.as_str(), "PDF/A part")?;
        let conformance = self.property(PDF_A_ID_NS, "conformance");
        let flavour_conformance = conformance.map_or_else(
            || String::from("none"),
            |property| property.value.as_str().to_ascii_lowercase(),
        );
        let flavour = ValidationFlavour::new("pdfa", part_number, flavour_conformance)?;
        Ok(Some(claim_from_property(
            XmpIdentificationKind::PdfA,
            &flavour,
            part,
            conformance,
            self.property(PDF_A_ID_NS, "rev"),
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
            None,
            self.property(PDF_UA_ID_NS, "rev"),
        )?))
    }

    fn wtpdf_claims(&self) -> Result<Vec<FlavourClaim>> {
        let mut claims = Vec::new();
        for property in self.properties.iter().filter(|property| {
            property.namespace.as_str() == PDF_D_NS && property.in_pdfd_declarations
        }) {
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
                    None,
                    None,
                )?);
            }
        }
        Ok(claims)
    }
}

#[derive(Clone, Debug)]
struct ElementFrame {
    namespace: BoundedText,
    prefix: Identifier,
    local: Identifier,
    array_kind: Option<Identifier>,
    xml_lang: Option<BoundedText>,
    text: String,
    previous_namespaces: BTreeMap<String, BoundedText>,
    in_pdfd_declarations: bool,
}

#[derive(Clone, Debug)]
struct XmpProperty {
    namespace: BoundedText,
    prefix: Identifier,
    local: Identifier,
    value: BoundedText,
    array_kind: Option<Identifier>,
    xml_lang: Option<BoundedText>,
    in_pdfd_declarations: bool,
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
    let mut xmp_limits = limits.clone();
    xmp_limits.max_stream_decode_bytes = limits.max_stream_decode_bytes.min(limits.max_xmp_bytes);
    let bytes = stream.decoded_bytes(&xmp_limits)?;
    enforce_xmp_len(bytes.len(), limits.max_xmp_bytes)?;
    Ok(Some((*metadata_key, bytes)))
}

/// Parses catalog XMP metadata without deciding validation profiles.
///
/// # Errors
///
/// Returns [`PdfvError`] when XMP byte/depth/count limits are exceeded.
pub(crate) fn parse_document_xmp(
    document: &crate::ParsedDocument,
    limits: &ResourceLimits,
    report_absent_metadata: bool,
) -> Result<XmpParseResult> {
    let mut warnings = Vec::new();
    let Some((object, bytes)) = catalog_xmp_bytes(document, limits, &mut warnings)? else {
        if report_absent_metadata {
            warnings.push(ValidationWarning::AutoDetection {
                message: BoundedText::unchecked("catalog metadata stream is missing"),
            });
        }
        return Ok(XmpParseResult {
            packet: None,
            parse_facts: Vec::new(),
            warnings,
        });
    };
    if document.is_encrypted() && !looks_like_xml(&bytes) {
        return Ok(XmpParseResult {
            packet: None,
            parse_facts: Vec::new(),
            warnings,
        });
    }
    let parser = XmpParser;
    match parser.parse_packet(object, &bytes, limits) {
        Ok(packet) => {
            warnings.extend(packet_warnings(&packet)?);
            let parse_facts = xmp_parse_facts(&packet, limits.max_parse_facts, &mut warnings)?;
            Ok(XmpParseResult {
                packet: Some(packet),
                parse_facts,
                warnings,
            })
        }
        Err(PdfvError::Parse(ParseError::LimitExceeded { limit })) => {
            Err(ParseError::LimitExceeded { limit }.into())
        }
        Err(error) => {
            let reason = BoundedText::new(error.to_string(), 512)
                .unwrap_or_else(|_| BoundedText::unchecked("XMP parse failed"));
            let fact = malformed_fact(reason.clone());
            let parse_facts = vec![ParseFact::Xmp { object, fact }];
            warnings.push(ValidationWarning::AutoDetection { message: reason });
            Ok(XmpParseResult {
                packet: None,
                parse_facts,
                warnings,
            })
        }
    }
}

fn malformed_fact(reason: BoundedText) -> XmpFact {
    if reason.as_str().contains("DTD")
        || reason.as_str().contains("entity")
        || reason.as_str().contains("forbidden")
    {
        XmpFact::HostileXmlRejected { reason }
    } else {
        XmpFact::Malformed { reason }
    }
}

fn looks_like_xml(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| byte == b'<')
}

fn select_compatible_profiles(
    profiles: Vec<ValidationProfile>,
    warnings: &mut Vec<ValidationWarning>,
) -> Result<Vec<ValidationProfile>> {
    let Some(first_group) = profiles
        .first()
        .map(|profile| compatibility_group(&profile.flavour))
    else {
        return Ok(Vec::new());
    };
    let mut selected = Vec::new();
    for profile in profiles {
        if compatibility_group(&profile.flavour) == first_group {
            selected.push(profile);
        } else {
            warnings.push(ValidationWarning::IncompatibleProfile {
                profile_id: profile.identity.id,
                reason: BoundedText::new(
                    "detected XMP claim is incompatible with the first selected PDF specification \
                     generation",
                    256,
                )?,
            });
        }
    }
    Ok(selected)
}

fn compatibility_group(flavour: &ValidationFlavour) -> &'static str {
    match (flavour.family.as_str(), flavour.part.get()) {
        ("pdfa", 1..=3) | ("pdfua", 1) => "pdf-1",
        _ => "pdf-2",
    }
}

fn xmp_parse_facts(
    packet: &XmpPacket,
    max_parse_facts: usize,
    warnings: &mut Vec<ValidationWarning>,
) -> Result<Vec<ParseFact>> {
    let retained = packet.facts.len().min(max_parse_facts);
    if packet.facts.len() > retained {
        warnings.push(ValidationWarning::ParseFactCapReached {
            cap: max_parse_facts,
        });
    }
    packet
        .facts
        .iter()
        .take(retained)
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
            XmpFact::DuplicateClaim { .. } => {
                Some("XMP metadata contains duplicate identification claims")
            }
            XmpFact::PacketParsed { .. }
            | XmpFact::FlavourClaim { .. }
            | XmpFact::PacketHeader { .. }
            | XmpFact::RdfProperty { .. } => None,
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
    conformance: Option<&XmpProperty>,
    rev: Option<&XmpProperty>,
) -> Result<FlavourClaim> {
    Ok(FlavourClaim {
        kind,
        flavour: flavour.clone(),
        part: flavour.part,
        conformance: conformance
            .map(|property| Identifier::new(property.value.as_str().to_ascii_uppercase()))
            .transpose()?,
        part_prefix: property.prefix.clone(),
        display_flavour: display_flavour(flavour)?,
        namespace_uri: property.namespace.clone(),
        property: property.local.clone(),
        rev: rev.map(|property| property.value.clone()),
        rev_prefix: rev.map(|property| property.prefix.clone()),
        conformance_prefix: conformance.map(|property| property.prefix.clone()),
    })
}

fn is_pdfd_declarations_context(
    stack: &[ElementFrame],
    namespace: &BoundedText,
    local: &Identifier,
) -> bool {
    (namespace.as_str() == PDF_D_NS && local.as_str() == "declarations")
        || stack.iter().any(|frame| {
            frame.namespace.as_str() == PDF_D_NS && frame.local.as_str() == "declarations"
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
        identifier_allow_empty(String::from_utf8_lossy(prefix).into_owned())?,
        Identifier::new(String::from_utf8_lossy(local).into_owned())?,
    ))
}

fn identifier_allow_empty(value: String) -> Result<Identifier> {
    if value.is_empty() {
        Ok(Identifier::unchecked(""))
    } else {
        Identifier::new(value).map_err(Into::into)
    }
}

fn is_identification_property(namespace: &BoundedText, local: &Identifier) -> bool {
    matches!(
        (namespace.as_str(), local.as_str()),
        (PDF_A_ID_NS, "part" | "conformance" | "rev")
            | (PDF_UA_ID_NS, "part" | "rev" | "amd" | "corr")
            | (PDF_D_NS, "conformsTo" | "declarations" | "value" | "li")
    )
}

fn is_rdf_property(namespace: &BoundedText, local: &Identifier) -> bool {
    let namespace = namespace.as_str();
    namespace != RDF_NS
        && namespace != XML_NS
        && !namespace.is_empty()
        && !matches!(
            local.as_str(),
            "RDF" | "Description" | "Alt" | "Bag" | "Seq" | "li"
        )
}

fn xpacket_attr(content: &str, name: &str, max_len: usize) -> Result<Option<BoundedText>> {
    let Some(start) = content.find(name) else {
        return Ok(None);
    };
    let after_name = content
        .get(start.saturating_add(name.len())..)
        .unwrap_or_default();
    let Some(after_eq) = after_name.trim_start().strip_prefix('=') else {
        return Ok(None);
    };
    let after_eq = after_eq.trim_start();
    let Some(quote) = after_eq
        .chars()
        .next()
        .filter(|quote| matches!(quote, '"' | '\''))
    else {
        return Ok(None);
    };
    let value_start = quote.len_utf8();
    let Some(rest) = after_eq.get(value_start..) else {
        return Ok(None);
    };
    let Some(end) = rest.find(quote) else {
        return Ok(None);
    };
    Ok(Some(BoundedText::new(
        rest.get(..end).unwrap_or_default().to_owned(),
        max_len,
    )?))
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

    fn utf16_le_packet(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
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
      <pdfd:declarations>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#reuse1.0</pdfd:conformsTo>
      </pdfd:declarations>
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
    fn test_should_preserve_multiple_wtpdf_declarations() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfd="http://pdfa.org/declarations/">
      <pdfd:declarations>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#reuse1.0</pdfd:conformsTo>
        <pdfd:conformsTo>http://pdfa.org/declarations/wtpdf#accessibility1.0</pdfd:conformsTo>
      </pdfd:declarations>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;
        let flavours = packet
            .identification
            .iter()
            .map(|claim| claim.display_flavour.as_str())
            .collect::<Vec<_>>();

        assert!(flavours.contains(&"wtpdf-1-0-reuse"));
        assert!(flavours.contains(&"wtpdf-1-0-accessibility"));
        Ok(())
    }

    #[test]
    fn test_should_extract_bounded_rdf_array_and_qualifier_facts() -> crate::Result<()> {
        let xml = br#"<?xpacket begin="" bytes="not-allowed"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"
                     xmlns:pdf="http://ns.adobe.com/pdf/1.3/"
                     xmlns:xmp="http://ns.adobe.com/xap/1.0/"
                     pdf:Producer="pdfv">
      <dc:title>
        <rdf:Alt>
          <rdf:li xml:lang="x-default">Bounded Title</rdf:li>
        </rdf:Alt>
      </dc:title>
      <dc:creator>
        <rdf:Seq>
          <rdf:li>Alice</rdf:li>
        </rdf:Seq>
      </dc:creator>
      <xmp:CreatorTool>unit-test</xmp:CreatorTool>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::PacketHeader {
                bytes: Some(bytes),
                encoding: None,
                actual_encoding,
            } if bytes.as_str() == "not-allowed" && actual_encoding.as_str() == "UTF-8"
        )));
        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::RdfProperty {
                namespace_uri,
                prefix,
                name,
                value: Some(value),
                array_kind: Some(array_kind),
                xml_lang: Some(xml_lang),
            } if namespace_uri.as_str() == "http://purl.org/dc/elements/1.1/"
                && prefix.as_str() == "dc"
                && name.as_str() == "title"
                && value.as_str() == "Bounded Title"
                && array_kind.as_str() == "Alt"
                && xml_lang.as_str() == "x-default"
        )));
        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::RdfProperty {
                namespace_uri,
                name,
                value: Some(value),
                ..
            } if namespace_uri.as_str() == "http://ns.adobe.com/pdf/1.3/"
                && name.as_str() == "Producer"
                && value.as_str() == "pdfv"
        )));
        Ok(())
    }

    #[test]
    fn test_should_surface_utf16_xmp_actual_encoding() -> crate::Result<()> {
        let xml = r#"<?xpacket begin=""?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="2"/>
  </rdf:RDF>
</x:xmpmeta>"#;
        let packet =
            XmpParser.parse_packet(key(), &utf16_le_packet(xml), &ResourceLimits::default())?;

        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::PacketHeader {
                actual_encoding,
                ..
            } if actual_encoding.as_str() == "UTF-16LE"
        )));
        assert!(packet.identification.iter().any(|claim| {
            claim.display_flavour.as_str() == "pdfa-2"
                && claim.kind == super::XmpIdentificationKind::PdfA
        }));
        Ok(())
    }

    #[test]
    fn test_should_extract_extension_schema_container_facts() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaExtension="http://www.aiim.org/pdfa/ns/extension/"
                     xmlns:pdfaSchema="http://www.aiim.org/pdfa/ns/schema#">
      <pdfaExtension:schemas>
        <rdf:Bag>
          <rdf:li rdf:parseType="Resource">
            <pdfaSchema:schema>Example</pdfaSchema:schema>
            <pdfaSchema:namespaceURI>https://example.test/ns/</pdfaSchema:namespaceURI>
            <pdfaSchema:prefix>ex</pdfaSchema:prefix>
          </rdf:li>
        </rdf:Bag>
      </pdfaExtension:schemas>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::RdfProperty {
                namespace_uri,
                prefix,
                name,
                array_kind: Some(array_kind),
                ..
            } if namespace_uri.as_str() == "http://www.aiim.org/pdfa/ns/extension/"
                && prefix.as_str() == "pdfaExtension"
                && name.as_str() == "schemas"
                && array_kind.as_str() == "Bag"
        )));
        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::RdfProperty {
                namespace_uri,
                prefix,
                name,
                value: Some(value),
                ..
            } if namespace_uri.as_str() == "http://www.aiim.org/pdfa/ns/schema#"
                && prefix.as_str() == "pdfaSchema"
                && name.as_str() == "namespaceURI"
                && value.as_str() == "https://example.test/ns/"
        )));
        Ok(())
    }

    #[test]
    fn test_should_report_duplicate_identification_claims() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="1"
                     pdfaid:conformance="B"/>
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="1"
                     pdfaid:conformance="B"/>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert_eq!(packet.identification.len(), 1);
        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::DuplicateClaim { property, .. } if property.as_str() == "part"
        )));
        Ok(())
    }

    #[test]
    fn test_should_report_conflicting_duplicate_identification_claims() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="1"/>
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="2"/>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert!(packet.facts.iter().any(|fact| matches!(
            fact,
            XmpFact::DuplicateClaim { property, .. } if property.as_str() == "part"
        )));
        Ok(())
    }

    #[test]
    fn test_should_restore_scoped_namespace_bindings() -> crate::Result<()> {
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:id="http://www.aiim.org/pdfa/ns/id/">
      <wrapper xmlns:id="http://www.aiim.org/pdfua/ns/id/">
        <id:part>2</id:part>
      </wrapper>
      <id:part>3</id:part>
      <id:conformance>U</id:conformance>
    </rdf:Description>
  </rdf:RDF>
</x:xmpmeta>"#;

        let packet = XmpParser.parse_packet(key(), xml, &ResourceLimits::default())?;

        assert!(packet.identification.iter().any(|claim| {
            claim.display_flavour.as_str() == "pdfa-3u"
                && claim.kind == super::XmpIdentificationKind::PdfA
        }));
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

    #[test]
    fn test_should_enforce_processing_instruction_cap() {
        let limits = ResourceLimits {
            max_xmp_processing_instructions: 1,
            ..ResourceLimits::default()
        };
        let xml = br#"<?xpacket begin=""?>
<?xpacket end="w"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#;

        let result = XmpParser.parse_packet(key(), xml, &limits);

        assert!(matches!(
            result,
            Err(crate::PdfvError::Parse(crate::ParseError::LimitExceeded {
                limit: "max_xmp_processing_instructions"
            }))
        ));
    }

    #[test]
    fn test_should_enforce_xmp_property_cap() {
        let limits = ResourceLimits {
            max_xmp_properties: 1,
            ..ResourceLimits::default()
        };
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
                     pdfaid:part="1"
                     pdfaid:conformance="B"/>
  </rdf:RDF>
</x:xmpmeta>"#;

        let result = XmpParser.parse_packet(key(), xml, &limits);

        assert!(matches!(
            result,
            Err(crate::PdfvError::Parse(crate::ParseError::LimitExceeded {
                limit: "max_xmp_properties"
            }))
        ));
    }
}
