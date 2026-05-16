# 18-xmp-metadata-flavour-design: XMP Metadata and Flavour Detection

Status: draft · Owner: pdfv · Depends on: [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md)

## 1. Purpose

This spec adds semantic XMP metadata parsing and veraPDF-style flavour detection. It owns XMP packet extraction, namespace-aware identification schema parsing, metadata validation facts, and auto profile selection. It does not own metadata repair.

veraPDF detects PDF/A, PDF/UA, and WTPDF flavours from catalog metadata before validation (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:163`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:171`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:178`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:182`). pdfv currently treats auto mode as default-profile selection, so this is required for profile parity.

## 2. Interface

```rust
pub struct XmpPacket {
    pub source_object: ObjectKey,
    pub bytes: BoundedBytes,
    pub namespaces: NamespaceMap,
    pub identification: Vec<FlavourClaim>,
    pub facts: Vec<XmpFact>,
}

pub trait XmpParser {
    fn parse_packet(&self, bytes: &[u8], limits: &ResourceLimits) -> Result<XmpPacket, PdfvError>;
}

pub struct FlavourDetector {
    profiles: Arc<dyn ProfileRepository + Send + Sync>,
}

impl FlavourDetector {
    pub fn detect(&self, document: &ParsedDocument, default: Option<ValidationFlavour>)
        -> Result<DetectedFlavours, PdfvError>;
}
```

`DetectedFlavours` records claims, selected profiles, skipped incompatible claims, parse warnings, and fallback reason.

## 3. XMP Scope

The parser must support enough RDF/XML to identify and validate PDF metadata claims:

- `pdfaid:part`, `pdfaid:conformance`, `pdfaid:rev`
- `pdfuaid:part`, `pdfuaid:rev`, `pdfuaid:amd`, `pdfuaid:corr`
- WTPDF declaration identifiers used by vendored WTPDF profiles
- basic Dublin Core and XMP properties needed by PDF/A metadata rules
- extension schema declarations needed by PDF/A and PDF/UA rules
- packet-level facts: malformed XML, missing xpacket wrapper, duplicate claims, unknown namespace prefix, invalid value type

The parser is not a generic XML DOM exposed to callers. It is a bounded semantic extractor for validation.

## 4. Auto Selection Behaviour

Auto mode:

1. Parse catalog metadata stream if present and decodable.
2. Extract all recognized flavour claims.
3. Map claims to built-in profiles from [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md).
4. If no recognized claim exists, use configured default if present.
5. If no claim and no default, return `Incomplete` with a warning rather than pretending PDF/A-1B was detected.
6. Skip incompatible profile groups with structured warnings.

Explicit profile selection bypasses auto detection but metadata facts remain available to rules.

## 5. Security and Resource Limits

XMP is hostile input:

- hard byte cap per metadata stream
- XML depth, element, attribute, namespace, text, and processing-instruction caps
- no external entity resolution
- no DTD processing
- no network or filesystem access
- reject or report invalid UTF-8 according to XML parser policy

If an XML crate is added, its latest stable version and security posture must be checked before implementation per AGENTS.md dependency policy. The design does not pin a crate here because this spec is crate-agnostic.

## 6. Invariants

- Auto-detected flavours are evidence-backed by XMP facts in the report.
- Multiple compatible claims can produce multiple profile reports.
- Missing or malformed XMP never panics and never hides parser facts.
- Password/decryption rules apply before XMP parsing: encrypted metadata is parsed only after successful decryption unless `/EncryptMetadata false` leaves it clear.
- Literal metadata strings are bounded in reports.

## 7. AGENTS.md binding

- Error Handling: XMP parse failures become structured warnings/facts where recoverable; fatal over-limit errors return `PdfvError`.
- Safety & Security: XML entity expansion, external resources, and unbounded text are forbidden.
- Type Design & API: namespace URIs, prefixes, flavour claims, and metadata facts are newtyped.
- Serialization: XMP facts serialize as `camelCase` and never include full metadata packets by default.
- Testing: malformed XML corpus, namespace alias cases, PDF/A/PDF/UA/WTPDF claim fixtures, and auto-selection integration tests.
- Performance: parse only catalog metadata streams required by selected profiles unless a rule explicitly links more metadata.
- Documentation: auto-selection docs explain fallback and warning semantics.

## 8. Cross-references

- ← Depends on: [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md)
- → Consumed by: [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md), [50-cli-design.md](./50-cli-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
