//! Bounded content-stream tokenization and operator summaries.

use std::collections::BTreeMap;

use crate::{
    BoundedText, ObjectKey, ObjectLocation, ParseError, PdfName, ResourceLimits, ValidationWarning,
};

const DEFAULT_MAX_OPERATOR_NAME_BYTES: usize = 32;
const DEFAULT_MAX_OPERAND_COUNT: usize = 256;
const DEFAULT_MAX_OPERAND_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_INLINE_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_GRAPHICS_STATE_DEPTH: u32 = 64;
const DEFAULT_MAX_CONTENT_STREAM_OPS: u64 = 100_000;
const DEFAULT_MAX_TYPE3_CHARPROC_STREAMS: u64 = 10_000;
const DEFAULT_MAX_TYPE3_CHARPROC_OPS: u64 = 10_000;

/// Default maximum content-stream operators parsed per stream.
pub(crate) const fn default_max_content_stream_ops() -> u64 {
    DEFAULT_MAX_CONTENT_STREAM_OPS
}

/// Default maximum content-stream operands retained for one operator.
pub(crate) const fn default_max_content_stream_operand_count() -> usize {
    DEFAULT_MAX_OPERAND_COUNT
}

/// Default maximum serialized operand bytes retained for one operator.
pub(crate) const fn default_max_content_stream_operand_bytes() -> usize {
    DEFAULT_MAX_OPERAND_BYTES
}

/// Default maximum content-stream operator name bytes.
pub(crate) const fn default_max_content_stream_operator_name_bytes() -> usize {
    DEFAULT_MAX_OPERATOR_NAME_BYTES
}

/// Default maximum inline-image data bytes scanned inside a content stream.
pub(crate) const fn default_max_inline_image_bytes() -> u64 {
    DEFAULT_MAX_INLINE_IMAGE_BYTES
}

/// Default maximum graphics-state stack depth tracked while parsing operators.
pub(crate) const fn default_max_graphics_state_depth() -> u32 {
    DEFAULT_MAX_GRAPHICS_STATE_DEPTH
}

/// Default maximum Type 3 charproc streams parsed per validation session.
pub(crate) const fn default_max_type3_charproc_streams() -> u64 {
    DEFAULT_MAX_TYPE3_CHARPROC_STREAMS
}

/// Default maximum operators parsed from one Type 3 charproc stream.
pub(crate) const fn default_max_type3_charproc_ops() -> u64 {
    DEFAULT_MAX_TYPE3_CHARPROC_OPS
}

/// Bounded parsed summary for one decoded content stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContentStreamSummary {
    /// Stream object key.
    pub source: ObjectKey,
    /// Number of operators seen before caps stopped parsing.
    pub operators_seen: u64,
    /// Operator facts retained for validation.
    pub facts: Vec<OperatorFact>,
    /// Marked-content spans retained for structure phases.
    pub marked_content: Vec<MarkedContentSpan>,
    /// Resource references found in operators.
    pub resource_uses: Vec<ResourceUse>,
    /// Recoverable parsing warnings.
    pub warnings: Vec<ValidationWarning>,
    /// Whether parsing stopped at an operator cap.
    pub truncated: bool,
}

impl ContentStreamSummary {
    /// Returns true when the stream has text operators.
    #[must_use]
    pub(crate) fn has_text(&self) -> bool {
        self.facts.iter().any(|fact| {
            matches!(
                fact,
                OperatorFact::TextObject { .. }
                    | OperatorFact::TextState { .. }
                    | OperatorFact::TextPosition { .. }
                    | OperatorFact::TextShow { .. }
            )
        })
    }

    /// Returns true when the stream has marked-content operators.
    #[must_use]
    pub(crate) fn has_marked_content(&self) -> bool {
        !self.marked_content.is_empty()
    }

    /// Returns true when the stream has inline-image operators.
    #[must_use]
    pub(crate) fn has_inline_image(&self) -> bool {
        self.facts
            .iter()
            .any(|fact| matches!(fact, OperatorFact::InlineImage { .. }))
    }

    /// Returns true when the stream has unknown operators.
    #[must_use]
    pub(crate) fn has_unknown_operators(&self) -> bool {
        self.facts
            .iter()
            .any(|fact| matches!(fact, OperatorFact::Unknown { .. }))
    }
}

/// Redacted and bounded operator fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperatorFact {
    /// Text object boundary operator.
    TextObject {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Text state operator.
    TextState {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Text positioning operator.
    TextPosition {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Text-show operator with only redacted byte count.
    TextShow {
        /// Operator name.
        op: OperatorName,
        /// Redacted shown operand bytes.
        bytes: BoundedBytes,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Marked-content operator.
    MarkedContent {
        /// Marked-content tag.
        tag: PdfName,
        /// Properties object when recoverable from operands.
        properties: Option<ObjectKey>,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Graphics-state operator.
    GraphicsState {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Color or shading operator.
    Color {
        /// Operator name.
        op: OperatorName,
        /// Bounded operand summary.
        operands: BoundedOperands,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// `XObject` invocation.
    XObjectInvoke {
        /// `XObject` resource name.
        name: PdfName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Inline image summary.
    InlineImage {
        /// Optional declared width.
        width: Option<u64>,
        /// Optional declared height.
        height: Option<u64>,
        /// Optional color-space name.
        color_space: Option<PdfName>,
        /// Optional bits per component.
        bits_per_component: Option<u64>,
        /// Declared filters.
        filters: Vec<PdfName>,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Path, clip, or paint operator.
    Path {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Compatibility-section operator.
    Compatibility {
        /// Operator name.
        op: OperatorName,
        /// Deterministic location.
        location: ObjectLocation,
    },
    /// Unknown or malformed recoverable operator.
    Unknown {
        /// Operator name.
        op: OperatorName,
        /// Operand count retained under caps.
        operand_count: u16,
        /// Deterministic location.
        location: ObjectLocation,
    },
}

impl OperatorFact {
    /// Returns this fact's operator name text.
    #[must_use]
    pub(crate) fn op_name(&self) -> &str {
        match self {
            Self::TextObject { op, .. }
            | Self::TextState { op, .. }
            | Self::TextPosition { op, .. }
            | Self::TextShow { op, .. }
            | Self::GraphicsState { op, .. }
            | Self::Color { op, .. }
            | Self::Path { op, .. }
            | Self::Compatibility { op, .. }
            | Self::Unknown { op, .. } => op.as_str(),
            Self::MarkedContent { .. } => "markedContent",
            Self::XObjectInvoke { .. } => "Do",
            Self::InlineImage { .. } => "BI",
        }
    }

    /// Returns the validation-model family for this operator fact.
    #[must_use]
    pub(crate) fn family(&self) -> &'static str {
        match self {
            Self::TextObject { .. } | Self::TextState { .. } | Self::TextPosition { .. } => "text",
            Self::TextShow { .. } => "textShow",
            Self::MarkedContent { .. } => "markedContent",
            Self::GraphicsState { .. } => "graphicsState",
            Self::Color { .. } => "color",
            Self::XObjectInvoke { .. } => "xObject",
            Self::InlineImage { .. } => "inlineImage",
            Self::Path { .. } => "path",
            Self::Compatibility { .. } => "compatibility",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// Returns this fact's location.
    #[must_use]
    pub(crate) fn location(&self) -> &ObjectLocation {
        match self {
            Self::TextObject { location, .. }
            | Self::TextState { location, .. }
            | Self::TextPosition { location, .. }
            | Self::TextShow { location, .. }
            | Self::MarkedContent { location, .. }
            | Self::GraphicsState { location, .. }
            | Self::Color { location, .. }
            | Self::XObjectInvoke { location, .. }
            | Self::InlineImage { location, .. }
            | Self::Path { location, .. }
            | Self::Compatibility { location, .. }
            | Self::Unknown { location, .. } => location,
        }
    }

    /// Returns true when this is an unknown operator.
    #[must_use]
    pub(crate) fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    /// Returns the bounded operand count associated with the fact.
    #[must_use]
    pub(crate) fn operand_count(&self) -> u16 {
        match self {
            Self::Unknown { operand_count, .. } => *operand_count,
            Self::Color { operands, .. } => operands.count,
            _ => 0,
        }
    }
}

/// Bounded operator name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OperatorName(BoundedText);

impl OperatorName {
    fn new(bytes: &[u8], limits: &ResourceLimits) -> Result<Self, ParseError> {
        if bytes.is_empty() || bytes.len() > limits.max_content_stream_operator_name_bytes {
            return Err(ParseError::LimitExceeded {
                limit: "max_content_stream_operator_name_bytes",
            });
        }
        Ok(Self(BoundedText::unchecked(
            String::from_utf8_lossy(bytes).into_owned(),
        )))
    }

    /// Returns the operator name text.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Bounded redacted byte count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoundedBytes {
    /// Redacted byte count.
    pub bytes: u64,
}

/// Bounded operand summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoundedOperands {
    /// Operand count.
    pub count: u16,
    /// Serialized operand bytes retained by the parser.
    pub bytes: u64,
}

/// Marked-content span summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkedContentSpan {
    /// Marked-content tag.
    pub tag: PdfName,
    /// Nesting depth at the start operator.
    pub nesting_depth: u32,
    /// Inline marked-content id from a property dictionary.
    pub mcid: Option<i64>,
    /// Optional properties object.
    pub properties: Option<ObjectKey>,
    /// Optional named property-list resource.
    pub properties_name: Option<PdfName>,
    /// Deterministic location.
    pub location: ObjectLocation,
}

/// Resource-use summary emitted for Phase 16 resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceUse {
    /// Resource family.
    pub family: ResourceFamily,
    /// Resource name.
    pub name: PdfName,
    /// Operator that referenced the resource.
    pub operator: OperatorName,
    /// Deterministic location.
    pub location: ObjectLocation,
}

/// Content-stream resource family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceFamily {
    /// Font resource from `Tf`.
    Font,
    /// Color-space resource from `CS` or `cs`.
    ColorSpace,
    /// External graphics state resource from `gs`.
    ExtGState,
    /// Shading resource from `sh`.
    Shading,
    /// `XObject` resource from `Do`.
    XObject,
}

impl ResourceFamily {
    /// Returns the validation family name.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Font => "font",
            Self::ColorSpace => "colorSpace",
            Self::ExtGState => "extGState",
            Self::Shading => "shading",
            Self::XObject => "xObject",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operand {
    Name(PdfName),
    Integer(i64),
    Bytes(u64),
    Composite(Vec<u8>),
    Keyword(Vec<u8>),
}

impl Operand {
    fn byte_len(&self) -> u64 {
        match self {
            Self::Name(name) => u64::try_from(name.as_bytes().len()).unwrap_or(u64::MAX),
            Self::Integer(value) => u64::try_from(value.to_string().len()).unwrap_or(u64::MAX),
            Self::Bytes(bytes) => *bytes,
            Self::Composite(bytes) | Self::Keyword(bytes) => {
                u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Operand(Operand),
    Operator(OperatorName, u64),
}

/// Parses decoded content-stream bytes into a bounded summary.
pub(crate) fn summarize_content_stream(
    source: ObjectKey,
    location_path: &str,
    decoded: &[u8],
    limits: &ResourceLimits,
) -> Result<ContentStreamSummary, ParseError> {
    let mut parser = ContentTokenizer::new(decoded, limits);
    let mut operands = Vec::new();
    let mut operand_bytes = 0_u64;
    let mut summary = ContentStreamSummary {
        source,
        operators_seen: 0,
        facts: Vec::new(),
        marked_content: Vec::new(),
        resource_uses: Vec::new(),
        warnings: Vec::new(),
        truncated: false,
    };
    let mut graphics_depth = 0_u32;
    let mut marked_depth = 0_u32;

    while let Some(token) = parser.next_token()? {
        match token {
            Token::Operand(operand) => push_operand(
                &mut operands,
                &mut operand_bytes,
                operand,
                limits,
                &mut summary.warnings,
            )?,
            Token::Operator(op, offset) => {
                if summary.operators_seen >= limits.max_content_stream_ops {
                    summary.truncated = true;
                    summary.warnings.push(ValidationWarning::General {
                        message: BoundedText::unchecked("content stream operator cap reached"),
                    });
                    break;
                }
                summary.operators_seen = summary.operators_seen.checked_add(1).ok_or(
                    ParseError::ArithmeticOverflow {
                        context: "content stream operator count",
                    },
                )?;
                let location =
                    operator_location(source, location_path, summary.operators_seen, offset);
                classify_operator(
                    op,
                    &operands,
                    operand_bytes,
                    location,
                    &mut parser,
                    &mut graphics_depth,
                    &mut marked_depth,
                    &mut summary,
                    limits,
                )?;
                operands.clear();
                operand_bytes = 0;
            }
        }
    }
    if !operands.is_empty() {
        summary.warnings.push(ValidationWarning::General {
            message: BoundedText::unchecked("content stream ended with dangling operands"),
        });
    }
    Ok(summary)
}

fn push_operand(
    operands: &mut Vec<Operand>,
    operand_bytes: &mut u64,
    operand: Operand,
    limits: &ResourceLimits,
    warnings: &mut Vec<ValidationWarning>,
) -> Result<(), ParseError> {
    let next_bytes =
        operand_bytes
            .checked_add(operand.byte_len())
            .ok_or(ParseError::ArithmeticOverflow {
                context: "content stream operand bytes",
            })?;
    if operands.len() >= limits.max_content_stream_operand_count
        || usize::try_from(next_bytes).unwrap_or(usize::MAX)
            > limits.max_content_stream_operand_bytes
    {
        operands.clear();
        *operand_bytes = 0;
        warnings.push(ValidationWarning::General {
            message: BoundedText::unchecked("content stream operand cap reached"),
        });
        return Ok(());
    }
    operands.push(operand);
    *operand_bytes = next_bytes;
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "operator classification is a flat PDF operator dispatch table"
)]
fn classify_operator(
    op: OperatorName,
    operands: &[Operand],
    operand_bytes: u64,
    location: ObjectLocation,
    parser: &mut ContentTokenizer<'_>,
    graphics_depth: &mut u32,
    marked_depth: &mut u32,
    summary: &mut ContentStreamSummary,
    limits: &ResourceLimits,
) -> Result<(), ParseError> {
    let op_name = op.as_str();
    match op_name {
        "BI" => {
            let inline = parser.parse_inline_image(limits)?;
            summary.facts.push(OperatorFact::InlineImage {
                width: inline.width,
                height: inline.height,
                color_space: inline.color_space,
                bits_per_component: inline.bits_per_component,
                filters: inline.filters,
                location,
            });
        }
        "BT" | "ET" => summary
            .facts
            .push(OperatorFact::TextObject { op, location }),
        "Tc" | "Tw" | "Tz" | "TL" | "Tf" | "Tr" | "Ts" => {
            if op_name == "Tf"
                && let Some(name) = first_name_operand(operands)
            {
                summary.resource_uses.push(ResourceUse {
                    family: ResourceFamily::Font,
                    name,
                    operator: op.clone(),
                    location: location.clone(),
                });
            }
            summary.facts.push(OperatorFact::TextState { op, location });
        }
        "Td" | "TD" | "Tm" | "T*" => {
            summary
                .facts
                .push(OperatorFact::TextPosition { op, location });
        }
        "Tj" | "TJ" | "'" | "\"" => summary.facts.push(OperatorFact::TextShow {
            op,
            bytes: BoundedBytes {
                bytes: operand_bytes,
            },
            location,
        }),
        "BMC" | "BDC" | "MP" | "DP" => {
            let tag = match first_name_operand(operands) {
                Some(tag) => tag,
                None => unknown_pdf_name(limits)?,
            };
            let properties = properties_reference(operands);
            let properties_name = properties_name(operands);
            let mcid = inline_mcid(operands);
            if matches!(op_name, "BMC" | "BDC") {
                *marked_depth = marked_depth.saturating_add(1);
                summary.marked_content.push(MarkedContentSpan {
                    tag: tag.clone(),
                    nesting_depth: *marked_depth,
                    mcid,
                    properties,
                    properties_name,
                    location: location.clone(),
                });
            }
            summary.facts.push(OperatorFact::MarkedContent {
                tag,
                properties,
                location,
            });
        }
        "EMC" => {
            *marked_depth = marked_depth.saturating_sub(1);
            summary.facts.push(OperatorFact::MarkedContent {
                tag: PdfName::new(b"EMC".to_vec(), limits)?,
                properties: None,
                location,
            });
        }
        "q" => {
            if *graphics_depth >= limits.max_graphics_state_depth {
                summary.warnings.push(ValidationWarning::General {
                    message: BoundedText::unchecked("graphics state depth cap reached"),
                });
            } else {
                *graphics_depth = graphics_depth.saturating_add(1);
            }
            summary
                .facts
                .push(OperatorFact::GraphicsState { op, location });
        }
        "Q" => {
            *graphics_depth = graphics_depth.saturating_sub(1);
            summary
                .facts
                .push(OperatorFact::GraphicsState { op, location });
        }
        "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" => {
            summary
                .facts
                .push(OperatorFact::GraphicsState { op, location });
        }
        "gs" => {
            if let Some(name) = first_name_operand(operands) {
                summary.resource_uses.push(ResourceUse {
                    family: ResourceFamily::ExtGState,
                    name,
                    operator: op.clone(),
                    location: location.clone(),
                });
            }
            summary
                .facts
                .push(OperatorFact::GraphicsState { op, location });
        }
        "CS" | "cs" => {
            if let Some(name) = first_name_operand(operands) {
                summary.resource_uses.push(ResourceUse {
                    family: ResourceFamily::ColorSpace,
                    name,
                    operator: op.clone(),
                    location: location.clone(),
                });
            }
            push_color_fact(summary, op, operands, operand_bytes, location)?;
        }
        "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => {
            push_color_fact(summary, op, operands, operand_bytes, location)?;
        }
        "sh" => {
            if let Some(name) = first_name_operand(operands) {
                summary.resource_uses.push(ResourceUse {
                    family: ResourceFamily::Shading,
                    name,
                    operator: op.clone(),
                    location: location.clone(),
                });
            }
            push_color_fact(summary, op, operands, operand_bytes, location)?;
        }
        "Do" => {
            if let Some(name) = first_name_operand(operands) {
                summary.resource_uses.push(ResourceUse {
                    family: ResourceFamily::XObject,
                    name: name.clone(),
                    operator: op.clone(),
                    location: location.clone(),
                });
                summary
                    .facts
                    .push(OperatorFact::XObjectInvoke { name, location });
            } else {
                push_unknown(summary, op, operands, location)?;
            }
        }
        "m" | "l" | "c" | "v" | "y" | "h" | "re" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*"
        | "b" | "b*" | "n" | "W" | "W*" => {
            summary.facts.push(OperatorFact::Path { op, location });
        }
        "BX" | "EX" => summary
            .facts
            .push(OperatorFact::Compatibility { op, location }),
        _ => push_unknown(summary, op, operands, location)?,
    }
    Ok(())
}

fn push_color_fact(
    summary: &mut ContentStreamSummary,
    op: OperatorName,
    operands: &[Operand],
    operand_bytes: u64,
    location: ObjectLocation,
) -> Result<(), ParseError> {
    let count = u16::try_from(operands.len()).map_err(|_| ParseError::LimitExceeded {
        limit: "max_content_stream_operand_count",
    })?;
    summary.facts.push(OperatorFact::Color {
        op,
        operands: BoundedOperands {
            count,
            bytes: operand_bytes,
        },
        location,
    });
    Ok(())
}

fn push_unknown(
    summary: &mut ContentStreamSummary,
    op: OperatorName,
    operands: &[Operand],
    location: ObjectLocation,
) -> Result<(), ParseError> {
    let operand_count = u16::try_from(operands.len()).map_err(|_| ParseError::LimitExceeded {
        limit: "max_content_stream_operand_count",
    })?;
    summary.facts.push(OperatorFact::Unknown {
        op,
        operand_count,
        location,
    });
    Ok(())
}

fn first_name_operand(operands: &[Operand]) -> Option<PdfName> {
    operands.iter().find_map(|operand| match operand {
        Operand::Name(name) => Some(name.clone()),
        _ => None,
    })
}

fn properties_name(operands: &[Operand]) -> Option<PdfName> {
    operands
        .iter()
        .filter_map(|operand| match operand {
            Operand::Name(name) => Some(name.clone()),
            _ => None,
        })
        .nth(1)
}

fn inline_mcid(operands: &[Operand]) -> Option<i64> {
    operands.iter().find_map(|operand| match operand {
        Operand::Composite(bytes) => parse_mcid_from_dictionary(bytes),
        _ => None,
    })
}

fn parse_mcid_from_dictionary(bytes: &[u8]) -> Option<i64> {
    let needle = b"/MCID";
    let start = bytes
        .windows(needle.len())
        .position(|window| window == needle)?;
    let mut pos = start.saturating_add(needle.len());
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos = pos.saturating_add(1);
    }
    let number_start = pos;
    if bytes.get(pos) == Some(&b'-') {
        pos = pos.saturating_add(1);
    }
    while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
        pos = pos.saturating_add(1);
    }
    if pos == number_start || bytes.get(number_start..pos) == Some(&b"-"[..]) {
        return None;
    }
    std::str::from_utf8(bytes.get(number_start..pos)?)
        .ok()?
        .parse::<i64>()
        .ok()
}

fn properties_reference(operands: &[Operand]) -> Option<ObjectKey> {
    let [
        ..,
        Operand::Integer(number),
        Operand::Integer(generation),
        Operand::Keyword(keyword),
    ] = operands
    else {
        return None;
    };
    if keyword.as_slice() != b"R" {
        return None;
    }
    let number = u32::try_from(*number)
        .ok()
        .and_then(std::num::NonZeroU32::new)?;
    let generation = u16::try_from(*generation).ok()?;
    Some(ObjectKey { number, generation })
}

fn operator_location(
    source: ObjectKey,
    location_path: &str,
    ordinal: u64,
    offset: u64,
) -> ObjectLocation {
    ObjectLocation {
        object: Some(source),
        offset: Some(offset),
        path: Some(BoundedText::unchecked(format!(
            "{location_path}/operator[{ordinal}]"
        ))),
    }
}

fn unknown_pdf_name(limits: &ResourceLimits) -> Result<PdfName, ParseError> {
    PdfName::new(b"Unknown".to_vec(), limits)
}

#[derive(Debug)]
struct InlineImageSummary {
    width: Option<u64>,
    height: Option<u64>,
    color_space: Option<PdfName>,
    bits_per_component: Option<u64>,
    filters: Vec<PdfName>,
}

struct ContentTokenizer<'a> {
    bytes: &'a [u8],
    pos: usize,
    limits: &'a ResourceLimits,
}

impl<'a> ContentTokenizer<'a> {
    fn new(bytes: &'a [u8], limits: &'a ResourceLimits) -> Self {
        Self {
            bytes,
            pos: 0,
            limits,
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_ws_and_comments();
        if self.pos >= self.bytes.len() {
            return Ok(None);
        }
        let start = self.pos;
        let token = match self.current_byte() {
            Some(b'/') => {
                self.pos = self.pos.saturating_add(1);
                let name_start = self.pos;
                self.consume_until_delimiter();
                let name_bytes =
                    self.bytes
                        .get(name_start..self.pos)
                        .ok_or(ParseError::ArithmeticOverflow {
                            context: "content stream name bounds",
                        })?;
                let bytes = decode_name(name_bytes, self.limits)?;
                Token::Operand(Operand::Name(PdfName::new(bytes, self.limits)?))
            }
            Some(b'(') => Token::Operand(Operand::Bytes(self.consume_literal_string()?)),
            Some(b'<') if self.peek_byte(1) != Some(b'<') => {
                Token::Operand(Operand::Bytes(self.consume_hex_string()?))
            }
            Some(b'[') => Token::Operand(Operand::Bytes(self.consume_composite()?)),
            Some(b'<') if self.peek_byte(1) == Some(b'<') => {
                Token::Operand(Operand::Composite(self.consume_composite_bytes()?))
            }
            _ => self.consume_regular_token(start)?,
        };
        Ok(Some(token))
    }

    fn consume_regular_token(&mut self, start: usize) -> Result<Token, ParseError> {
        self.consume_until_delimiter();
        let slice = self
            .bytes
            .get(start..self.pos)
            .ok_or(ParseError::ArithmeticOverflow {
                context: "content stream token bounds",
            })?;
        if is_keyword_operand(slice) {
            return Ok(Token::Operand(Operand::Keyword(slice.to_vec())));
        }
        if let Some(integer) = parse_integer(slice) {
            return Ok(Token::Operand(Operand::Integer(integer)));
        }
        if is_number_like(slice) {
            return Ok(Token::Operand(Operand::Bytes(
                u64::try_from(slice.len()).unwrap_or(u64::MAX),
            )));
        }
        Ok(Token::Operator(
            OperatorName::new(slice, self.limits)?,
            u64::try_from(start).map_err(|_| ParseError::ArithmeticOverflow {
                context: "content stream offset",
            })?,
        ))
    }

    fn parse_inline_image(
        &mut self,
        limits: &ResourceLimits,
    ) -> Result<InlineImageSummary, ParseError> {
        let mut values = BTreeMap::new();
        let mut key: Option<PdfName> = None;
        while let Some(token) = self.next_token()? {
            match token {
                Token::Operator(op, _) if op.as_str() == "ID" => break,
                Token::Operand(Operand::Name(name)) if key.is_none() => key = Some(name),
                Token::Operand(operand) => {
                    if let Some(name) = key.take() {
                        values.insert(
                            String::from_utf8_lossy(name.as_bytes()).into_owned(),
                            operand,
                        );
                    }
                }
                Token::Operator(op, _) => {
                    if let Some(name) = key.take() {
                        values.insert(
                            name_to_string(&name),
                            Operand::Keyword(op.as_str().as_bytes().to_vec()),
                        );
                    }
                }
            }
        }
        self.consume_inline_image_data(limits)?;
        Ok(InlineImageSummary {
            width: inline_u64(&values, "W").or_else(|| inline_u64(&values, "Width")),
            height: inline_u64(&values, "H").or_else(|| inline_u64(&values, "Height")),
            color_space: inline_name(&values, "CS").or_else(|| inline_name(&values, "ColorSpace")),
            bits_per_component: inline_u64(&values, "BPC")
                .or_else(|| inline_u64(&values, "BitsPerComponent")),
            filters: inline_filters(&values),
        })
    }

    fn consume_inline_image_data(&mut self, limits: &ResourceLimits) -> Result<(), ParseError> {
        if matches!(self.current_byte(), Some(byte) if is_whitespace(byte)) {
            self.pos = self.pos.saturating_add(1);
        }
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let scanned = u64::try_from(self.pos.saturating_sub(start)).map_err(|_| {
                ParseError::ArithmeticOverflow {
                    context: "inline image bytes",
                }
            })?;
            if scanned > limits.max_inline_image_bytes {
                return Err(ParseError::LimitExceeded {
                    limit: "max_inline_image_bytes",
                });
            }
            if self.bytes.get(self.pos..self.pos.saturating_add(2)) == Some(b"EI")
                && self
                    .pos
                    .checked_sub(1)
                    .and_then(|prev| self.bytes.get(prev))
                    .is_some_and(|byte| is_whitespace(*byte))
                && self
                    .bytes
                    .get(self.pos.saturating_add(2))
                    .is_none_or(|byte| is_whitespace(*byte) || is_delimiter(*byte))
            {
                self.pos = self.pos.saturating_add(2);
                return Ok(());
            }
            self.pos = self.pos.saturating_add(1);
        }
        Err(ParseError::Malformed {
            message: BoundedText::unchecked("inline image missing EI terminator"),
        })
    }

    fn consume_literal_string(&mut self) -> Result<u64, ParseError> {
        let start = self.pos;
        self.pos = self.pos.saturating_add(1);
        let mut depth = 1_u32;
        while self.pos < self.bytes.len() {
            match self.current_byte() {
                Some(b'\\') => self.pos = self.pos.saturating_add(2),
                Some(b'(') => {
                    depth = depth.checked_add(1).ok_or(ParseError::ArithmeticOverflow {
                        context: "literal string depth",
                    })?;
                    self.pos = self.pos.saturating_add(1);
                }
                Some(b')') => {
                    depth = depth.saturating_sub(1);
                    self.pos = self.pos.saturating_add(1);
                    if depth == 0 {
                        return checked_slice_len(start, self.pos, "literal string length");
                    }
                }
                Some(_) => self.pos = self.pos.saturating_add(1),
                None => break,
            }
        }
        Err(ParseError::Malformed {
            message: BoundedText::unchecked("unterminated literal string in content stream"),
        })
    }

    fn consume_hex_string(&mut self) -> Result<u64, ParseError> {
        let start = self.pos;
        self.pos = self.pos.saturating_add(1);
        while self.pos < self.bytes.len() {
            if self.current_byte() == Some(b'>') {
                self.pos = self.pos.saturating_add(1);
                return checked_slice_len(start, self.pos, "hex string length");
            }
            self.pos = self.pos.saturating_add(1);
        }
        Err(ParseError::Malformed {
            message: BoundedText::unchecked("unterminated hex string in content stream"),
        })
    }

    fn consume_composite(&mut self) -> Result<u64, ParseError> {
        let start = self.pos;
        self.consume_composite_range()?;
        checked_slice_len(start, self.pos, "composite operand length")
    }

    fn consume_composite_bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        let start = self.pos;
        self.consume_composite_range()?;
        let end = self.pos;
        let len = checked_slice_len(start, end, "dictionary operand length")?;
        if usize::try_from(len).unwrap_or(usize::MAX) > self.limits.max_content_stream_operand_bytes
        {
            return Err(ParseError::LimitExceeded {
                limit: "max_content_stream_operand_bytes",
            });
        }
        self.bytes
            .get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(ParseError::ArithmeticOverflow {
                context: "dictionary operand bounds",
            })
    }

    fn consume_composite_range(&mut self) -> Result<(), ParseError> {
        let mut stack = Vec::new();
        loop {
            match self.current_byte() {
                Some(b'[') => {
                    stack.push(b']');
                    self.pos = self.pos.saturating_add(1);
                }
                Some(b'<') if self.peek_byte(1) == Some(b'<') => {
                    stack.push(b'>');
                    self.pos = self.pos.saturating_add(2);
                }
                Some(b']') => {
                    if stack.pop() != Some(b']') {
                        return Err(malformed_content("mismatched array delimiter"));
                    }
                    self.pos = self.pos.saturating_add(1);
                    if stack.is_empty() {
                        return Ok(());
                    }
                }
                Some(b'>') if self.peek_byte(1) == Some(b'>') => {
                    if stack.pop() != Some(b'>') {
                        return Err(malformed_content("mismatched dictionary delimiter"));
                    }
                    self.pos = self.pos.saturating_add(2);
                    if stack.is_empty() {
                        return Ok(());
                    }
                }
                Some(b'(') => {
                    self.consume_literal_string()?;
                }
                Some(b'<') => {
                    self.consume_hex_string()?;
                }
                Some(_) => self.pos = self.pos.saturating_add(1),
                None => return Err(malformed_content("unterminated composite operand")),
            }
            if stack.len() > usize::try_from(self.limits.max_object_depth).unwrap_or(usize::MAX) {
                return Err(ParseError::LimitExceeded {
                    limit: "max_object_depth",
                });
            }
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.current_byte().is_some_and(is_whitespace) {
                self.pos = self.pos.saturating_add(1);
            }
            if self.current_byte() != Some(b'%') {
                break;
            }
            while self
                .current_byte()
                .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
            {
                self.pos = self.pos.saturating_add(1);
            }
        }
    }

    fn consume_until_delimiter(&mut self) {
        while self
            .current_byte()
            .is_some_and(|byte| !is_whitespace(byte) && !is_delimiter(byte))
        {
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn current_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos.saturating_add(offset)).copied()
    }
}

fn inline_u64(values: &BTreeMap<String, Operand>, key: &str) -> Option<u64> {
    match values.get(key) {
        Some(Operand::Integer(value)) => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn inline_name(values: &BTreeMap<String, Operand>, key: &str) -> Option<PdfName> {
    match values.get(key) {
        Some(Operand::Name(value)) => Some(value.clone()),
        _ => None,
    }
}

fn inline_filters(values: &BTreeMap<String, Operand>) -> Vec<PdfName> {
    ["F", "Filter"]
        .iter()
        .filter_map(|key| inline_name(values, key))
        .collect()
}

fn name_to_string(name: &PdfName) -> String {
    String::from_utf8_lossy(name.as_bytes()).into_owned()
}

fn decode_name(bytes: &[u8], limits: &ResourceLimits) -> Result<Vec<u8>, ParseError> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut pos = 0_usize;
    while pos < bytes.len() {
        if bytes.get(pos) == Some(&b'#')
            && let (Some(high), Some(low)) = (
                bytes.get(pos.saturating_add(1)),
                bytes.get(pos.saturating_add(2)),
            )
            && let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low))
        {
            decoded.push(high.saturating_mul(16).saturating_add(low));
            pos = pos.saturating_add(3);
            continue;
        }
        if let Some(byte) = bytes.get(pos) {
            decoded.push(*byte);
        }
        pos = pos.saturating_add(1);
    }
    if decoded.len() > limits.max_name_bytes {
        return Err(ParseError::LimitExceeded {
            limit: "max_name_bytes",
        });
    }
    Ok(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_integer(bytes: &[u8]) -> Option<i64> {
    std::str::from_utf8(bytes).ok()?.parse::<i64>().ok()
}

fn is_number_like(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(f64::is_finite)
}

fn is_keyword_operand(bytes: &[u8]) -> bool {
    matches!(bytes, b"true" | b"false" | b"null" | b"R")
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0x00 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn checked_slice_len(start: usize, end: usize, context: &'static str) -> Result<u64, ParseError> {
    let len = end
        .checked_sub(start)
        .ok_or(ParseError::ArithmeticOverflow { context })?;
    u64::try_from(len).map_err(|_| ParseError::ArithmeticOverflow { context })
}

fn malformed_content(message: &'static str) -> ParseError {
    ParseError::Malformed {
        message: BoundedText::unchecked(message),
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::{OperatorFact, ResourceFamily, summarize_content_stream};
    use crate::{ObjectKey, ResourceLimits};

    fn key() -> ObjectKey {
        ObjectKey::new(NonZeroU32::MIN, 0)
    }

    #[test]
    fn test_should_summarize_text_marked_content_and_resource_uses() -> crate::Result<()> {
        let summary = summarize_content_stream(
            key(),
            "root/page[0]/contentStream[0]",
            b"BT /F1 12 Tf (secret text) Tj ET /Cs1 CS /Im1 Do /GS1 gs /Sh1 sh",
            &ResourceLimits::default(),
        )?;

        assert!(summary.has_text());
        assert_eq!(summary.resource_uses.len(), 5);
        assert!(
            summary
                .resource_uses
                .iter()
                .any(|resource| resource.family == ResourceFamily::Font)
        );
        assert!(
            summary.facts.iter().any(
                |fact| matches!(fact, OperatorFact::TextShow { bytes, .. } if bytes.bytes > 0)
            )
        );
        Ok(())
    }

    #[test]
    fn test_should_record_unknown_operators_without_failing() -> crate::Result<()> {
        let summary = summarize_content_stream(
            key(),
            "root/page[0]/contentStream[0]",
            b"1 2 3 WeirdOp",
            &ResourceLimits::default(),
        )?;

        assert!(summary.has_unknown_operators());
        assert_eq!(summary.operators_seen, 1);
        Ok(())
    }

    #[test]
    fn test_should_stop_at_operator_cap() -> crate::Result<()> {
        let limits = ResourceLimits {
            max_content_stream_ops: 2,
            ..ResourceLimits::default()
        };
        let summary =
            summarize_content_stream(key(), "root/page[0]/contentStream[0]", b"q Q q Q", &limits)?;

        assert!(summary.truncated);
        assert_eq!(summary.operators_seen, 2);
        Ok(())
    }

    #[test]
    fn test_should_parse_inline_image_summary() -> crate::Result<()> {
        let summary = summarize_content_stream(
            key(),
            "root/page[0]/contentStream[0]",
            b"BI /W 2 /H 3 /BPC 8 /CS /RGB ID abc EI",
            &ResourceLimits::default(),
        )?;

        assert!(summary.facts.iter().any(|fact| matches!(
            fact,
            OperatorFact::InlineImage {
                width: Some(2),
                height: Some(3),
                ..
            }
        )));
        Ok(())
    }
}
