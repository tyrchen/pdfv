# 11-parser-core-design: Tolerant PDF Parser

Status: draft · Owner: pdfv · Depends on: [10-data-model.md](./10-data-model.md)

## 1. Purpose

The parser owns byte-level PDF syntax, COS object materialization, xref/trailer resolution, stream metadata, and parse facts. It does not own validation profiles, rule execution, report formatting, CLI output, or PDF rendering.

## 2. Interface

```rust
pub trait PdfSource: std::io::Read + std::io::Seek {}

pub struct ParsedDocument {
    catalog: Option<ObjectId>,
    objects: ObjectStore,
    trailers: Vec<Trailer>,
    parse_facts: Vec<ParseFact>,
    warnings: Vec<ValidationWarning>,
}

pub struct Parser {
    limits: ResourceLimits,
}

impl Parser {
    pub fn parse<R: PdfSource>(&self, source: R) -> Result<ParsedDocument, PdfvError>;
    pub fn parse_with_options<R: PdfSource>(
        &self,
        source: R,
        options: ParseOptions<'_>,
    ) -> Result<ParsedDocument, PdfvError>;
}
```

The first implementation uses `Read + Seek` and may add memory-mapped file support behind a feature later. `memmap2 0.9.10` is a candidate but must be optional because memory maps complicate hostile-file and platform behaviour.

## 3. Data structures

`CosObject` is an enum, not an untyped object wrapper:

```rust
pub enum CosObject {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(OrderedReal),
    Name(Name),
    String(PdfString),
    Array(Vec<ObjectRef>),
    Dictionary(Dictionary),
    Stream(StreamObject),
    Reference(ObjectKey),
}
```

`ObjectStore` stores indirect objects by `ObjectKey { number: NonZeroU32, generation: u16 }`. Direct objects live inline where possible. `StreamObject` stores dictionary, raw byte range, declared length, discovered length, EOL facts, and filter list; decoded stream access returns a bounded reader.

## 4. Behaviour

Header parsing is tolerant like veraPDF: search for `%PDF-`, record leading bytes and header offset, parse the version if available, and emit a fact if the header is malformed but recoverable (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/PDFParser.java:94`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/PDFParser.java:127`). The parser must not silently normalize away anomalies that validation rules need.

Stream parsing validates declared `/Length` against `endstream`; when invalid, it scans for `endstream` within bounded limits and records both declared and discovered lengths. This follows the veraPDF behaviour where stream parsing validates length and falls back to scanning (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:113`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:124`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:134`).

Xref parsing supports classic xref tables in M0 and records unsupported xref stream facts rather than crashing. Xref streams land in M1 because many modern PDFs require them for broad coverage.

Encrypted PDFs are detected and reported as `ValidationStatus::Encrypted` unless password-capable parsing is active. Password-capable parsing is scoped by [14-password-decryption-design.md](./14-password-decryption-design.md): Phase 9 supports only the Standard security handler revisions 2-4 and keeps password storage in redacted secret wrappers per AGENTS.md Cryptography & Secrets.

## 5. Resource limits

`ResourceLimits` includes:

- `max_file_bytes`
- `max_objects`
- `max_object_depth`
- `max_array_len`
- `max_dict_entries`
- `max_name_bytes`
- `max_string_bytes`
- `max_stream_declared_bytes`
- `max_stream_decode_bytes`
- `max_parse_facts`

All limits are enforced at parse boundary. Over-limit inputs return structured errors or validation warnings depending on recoverability.

## 6. AGENTS.md binding

- Error Handling: `ParseError` is a `thiserror` enum with `#[source]` fields; no `unwrap`/`expect` in production parser code.
- Safety & Security: `#![forbid(unsafe_code)]`, bounded collections, checked arithmetic, recursion limits, and no panics on user bytes.
- Type Design & API: domain newtypes for offsets, object keys, names, string byte caps, and non-zero object numbers.
- Serialization: parser facts serialize with `camelCase`.
- Testing: unit tests for token parsers; integration fixtures for malformed headers, bad xrefs, bad stream length; proptest and fuzz harnesses for byte-level tokenization.
- Logging & Observability: parser does not log user bytes; it returns facts and warnings. Tracing spans may include object key and offset.
- Performance: avoid eager stream decoding and unnecessary string allocation; use byte slices and `Cow<'_, str>` where text conversion is required.
- Documentation: public parser-facing types have doc comments and examples.

## 7. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md)
- → Consumed by: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
