# 15-parser-filter-source-parity-design: Parser Filter and Source Parity

Status: draft · Owner: pdfv · Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md)

## 1. Purpose

This spec extends the tolerant parser from a useful PDF/A validation spine toward veraPDF parser parity. It owns stream filter coverage, predictor handling, source storage strategy, incremental-update/xref-chain behaviour, and parser facts needed by broader validation profiles. It does not own validation-model object wrappers or report formatting.

The current parser deliberately supports a narrow set: classic xref tables, xref streams, object streams, FlateDecode, stream length facts, and Standard security handler decryption. veraPDF supports a wider filter registry and seekable source model (`vendors/veraPDF-parser/src/main/java/org/verapdf/as/filters/ASFilterFactory.java:52`, `vendors/veraPDF-parser/src/main/java/org/verapdf/cos/COSStream.java:157`).

## 2. Interface

```rust
pub trait PdfSource: Read + Seek {}

pub trait StreamDecoder {
    fn decode<'a>(
        &self,
        input: BoundedByteSource<'a>,
        params: DecodeParams<'a>,
        limits: &ResourceLimits,
    ) -> Result<DecodedStream, PdfvError>;
}

pub struct DecoderRegistry {
    decoders: BTreeMap<PdfName, Arc<dyn StreamDecoder + Send + Sync>>,
}

pub enum SourceStorage {
    Memory(Bytes),
    SpillFile(TempPath),
    Mmap(Arc<Mmap>),
}
```

`Parser` owns a `DecoderRegistry`. Defaults are pure Rust, deterministic, bounded, and available without global registration. Feature flags may gate heavyweight or rarely used decoders, but unsupported filters must produce structured facts/warnings rather than panics.

## 3. Required Filter Coverage

The parity target is:

| Filter | Required behaviour | Notes |
| --- | --- | --- |
| `FlateDecode`, `Fl` | zlib/raw deflate with PNG/TIFF predictors | Existing Flate support must gain `/DecodeParms` predictor handling. |
| `ASCIIHexDecode`, `AHx` | Decode ASCII hex with EOD and whitespace tolerance | Needed by many small conformance fixtures. |
| `ASCII85Decode`, `A85` | Decode ASCII85 with `~>` EOD and `z` shortcut | Must reject malformed groups under limits. |
| `LZWDecode`, `LZW` | LZW with early-change and predictor support | Pure Rust implementation preferred; dependency requires audit. |
| `RunLengthDecode`, `RL` | PDF run-length decoding | Simple bounded implementation in crate. |
| `Crypt` | Identity passthrough and named crypt filters from encryption dictionary | Must compose with [14-password-decryption-design.md](./14-password-decryption-design.md). |
| `DCTDecode`, `JPXDecode`, `JBIG2Decode`, `CCITTFaxDecode` | Byte-preserving metadata mode first; full image decode only if validation rules need pixels | PDF/A often needs image metadata, not rendered pixels. |

veraPDF's filter factory decodes ASCIIHex, RunLength, Flate with predictors, ASCII85, and LZW (`vendors/veraPDF-parser/src/main/java/org/verapdf/as/filters/ASFilterFactory.java:55`, `vendors/veraPDF-parser/src/main/java/org/verapdf/as/filters/ASFilterFactory.java:63`). pdfv parity requires at least those for decoded validation streams.

## 4. Source Storage

The current parser reads the entire file into memory before parsing. That is acceptable under a hard file cap, but veraPDF parity needs large-file handling without unbounded resident memory.

Rules:

- Inputs under `memory_source_threshold_bytes` may stay in memory.
- Larger file inputs use a seekable spill file or optional memory map.
- Memory maps are optional and never required for correctness.
- Decoded streams are not cached globally. A decoded stream may be cached per `ValidationSession` only if bounded by `max_decoded_cache_bytes`.
- Temporary files are removed deterministically; failures become warnings only after all validation-visible facts are preserved.

## 5. Xref and Incremental Updates

The parser must follow `startxref`, `/Prev`, and hybrid-reference chains in dependency order:

1. Parse the latest xref section or xref stream.
2. Follow `/Prev` with a depth and byte-offset cycle cap.
3. Merge object revisions with latest revision winning.
4. Preserve all trailer dictionaries and emit facts for inconsistent `/Size`, duplicate live entries, malformed free-list entries, hybrid xref streams, and post-EOF data.

veraPDF records header offset and tolerant stream-length facts rather than losing anomalies (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/PDFParser.java:113`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:205`). pdfv must apply the same principle to xref-chain anomalies.

## 6. Invariants

- Every decoded byte count is checked against `max_stream_decode_bytes`.
- Every decoder is streaming or chunked; no decoder may require multiplying input size without a preflight bound.
- Filter arrays and `/DecodeParms` arrays preserve order and cardinality.
- `Crypt` is applied at the PDF encryption layer before ordinary filters when the object is encrypted.
- Parser facts are stable enough for rules and reports; warnings are not the only observable output.

## 7. AGENTS.md binding

- Error Handling: decoder failures use `ParseError::UnsupportedFilter`, `ParseError::StreamDecode`, or a more specific `ParseError` variant; no panics on malformed bytes.
- Safety & Security: external stream bytes are hostile; all decode loops use checked arithmetic and byte-count caps.
- Type Design & API: decoder names, decode params, and source storage modes are explicit enums/newtypes.
- Testing: per-filter fixtures, chained-filter fixtures, predictor fixtures, malformed/EOD edge cases, and differential fixtures against vendored veraPDF where feasible.
- Performance: add Criterion benches for Flate predictor, ASCII85, LZW, object-stream expansion, and large-file source storage.
- Documentation: public decoder extension points document hostile-input expectations and unsupported image-pixel scope.

## 8. Cross-references

- ← Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md)
- → Consumed by: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
