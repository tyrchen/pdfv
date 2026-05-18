# M9 G5 XMP RDF Metadata Validation Snapshot

Status: Done | Owner: pdfv | Date: 2026-05-18

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
PDFV_PARITY_BASELINE_DIR=docs/reviews/m9-g4-resource-color-image-xobject-semantics make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g5-xmp-rdf-metadata-validation/`](./m9-g5-xmp-rdf-metadata-validation/)
for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G5 Summary

| Metric | Count |
| --- | ---: |
| Imported official rules | 6,749 |
| Remaining unsupported rules | 238 |
| Remaining unsupported clusters | 28 |
| Release-blocking unsupported rules | 93 |
| Release-blocking unsupported clusters | 19 |
| G5 metadata/XMP RDF clusters | 0 |
| G5 release-blocking clusters | 0 |

G5 removed every missing-property cluster assigned to XMP RDF metadata
validation. The implementation now retains bounded RDF property facts, array
container kinds, `xml:lang` qualifiers, packet-header attributes, Dublin Core
title/creator/description values, Adobe PDF/XMP Info-dictionary mirrors,
PDF/A extension-schema guards, WTPDF declaration facts, and PDF/UA
`x-default` language-alternative facts. Metadata rule lookups are scoped to
the source metadata stream, extension-schema prefix checks are namespace-aware,
and UTF-16/UTF-32 packets expose their observed XML encoding instead of being
reported as UTF-8. Reports retain bounded facts only and do not include full
XMP packets.

## Bound Coverage

All PDF/A profile rules are now lowered and bound after the G5 snapshot:

| Flavour | Bound / total |
| --- | ---: |
| pdfa-1a | 135 / 135 |
| pdfa-1b | 129 / 129 |
| pdfa-2a | 153 / 153 |
| pdfa-2b | 144 / 144 |
| pdfa-2u | 146 / 146 |
| pdfa-3a | 155 / 155 |
| pdfa-3b | 146 / 146 |
| pdfa-3u | 148 / 148 |
| pdfa-4 | 109 / 109 |
| pdfa-4e | 109 / 109 |
| pdfa-4f | 109 / 109 |

## Corpus

The Java-free parity corpus remains stable: 10 / 10 rows match, with 0 expected
drift and 0 unexpected drift. The generated XMP row continues to exercise
auto-detected PDF/A metadata selection and is classified as a match.

`corpus-agreement.json` records `liveOracleEnabled: false`; live veraPDF oracle
execution remains part of the later oracle-automation workstream. This G5
snapshot is the bounded Java-free evidence for XMP RDF metadata validation.

## Remaining Readiness Work

The remaining release-blocking unsupported clusters are assigned to G6
accessibility reconstruction and later oracle/readiness documentation work.
