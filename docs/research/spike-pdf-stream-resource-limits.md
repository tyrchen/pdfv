# Spike: PDF Stream Resource Limits

Status: Done · Owner: pdfv · Date: 2026-05-15

## Question

Which stream parsing and decoding limits should be part of the M0 public contract?

## Inputs Reviewed

- `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java`
- `vendors/veraPDF-parser/src/main/java/org/verapdf/cos/filters/COSFilterFlateDecode.java`
- Existing architecture study: [study-verapdf-validator-architecture.md](./study-verapdf-validator-architecture.md)
- Design targets: [../../specs/10-data-model.md](../../specs/10-data-model.md), [../../specs/11-parser-core-design.md](../../specs/11-parser-core-design.md), [../../specs/70-security.md](../../specs/70-security.md)

## Findings

veraPDF validates declared stream length, seeks to the expected `endstream`, then falls back to a bounded buffer scan when the declared length is wrong. It records compliance facts for `stream` and `endstream` keyword spacing rather than collapsing every anomaly into a fatal parse error.

Rust M0 should preserve that behavior but must add explicit resource caps at the parser boundary. The public `ResourceLimits` contract should include file bytes, object counts, nesting depth, collection sizes, name/string byte limits, declared stream bytes, decoded stream bytes, and retained parse fact count.

Decoded stream handling must be lazy and byte-counted. Even when Flate support is enabled later, report generation and rule checks should not eagerly materialize arbitrary decoded streams.

## Initial Defaults

The phase-1 contract pins conservative defaults:

- `max_file_bytes`: 256 MiB
- `max_stream_declared_bytes`: 128 MiB
- `max_stream_decode_bytes`: 256 MiB
- `max_parse_facts`: 100,000
- collection and text caps as encoded in `ResourceLimits`

These are public defaults, not hard upper bounds. Parser implementation phases may add compiled hard caps for safety.

## Spec Impact

[../../specs/10-data-model.md](../../specs/10-data-model.md) and [../../specs/11-parser-core-design.md](../../specs/11-parser-core-design.md) already list the required limit fields. No M0 scope reduction is required.
