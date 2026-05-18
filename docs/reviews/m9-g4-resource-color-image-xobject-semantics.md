# M9 G4 Resource, Color, Image, and XObject Semantics Snapshot

Status: Done | Owner: pdfv | Date: 2026-05-18

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g4-resource-color-image-xobject-semantics/`](./m9-g4-resource-color-image-xobject-semantics/)
for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G4 Summary

| Metric | Count |
| --- | ---: |
| Imported official rules | 6,749 |
| Remaining unsupported rules | 469 |
| Remaining unsupported clusters | 59 |
| Release-blocking unsupported rules | 322 |
| Release-blocking unsupported clusters | 49 |
| G4 resource/color/image/XObject clusters | 0 |
| G4 release-blocking clusters | 0 |

G4 removed every missing-property and missing-semantic-family cluster assigned
to effective resource, color, image, output-intent, ICC, ExtGState, and XObject
decision semantics. The implemented facts are bounded summaries: output profile
identity/reference checks, ICC output color tags, image color and mask metadata,
ExtGState blend/soft-mask/transfer keys, form/XObject forbidden-key checks, and
color-space component/output/tint/colorant facts. ICC byte payloads remain
report-safe and MD5-dependent comparisons return runtime `Incomplete` rather
than optimistic compliance when byte-level digest evidence is required.

## Bound Coverage

PDF/A profile bound-rule coverage improved after the G3 snapshot:

| Flavour | Bound / total |
| --- | ---: |
| pdfa-1a | 103 / 135 |
| pdfa-1b | 97 / 129 |
| pdfa-2a | 128 / 153 |
| pdfa-2b | 119 / 144 |
| pdfa-2u | 121 / 146 |
| pdfa-3a | 130 / 155 |
| pdfa-3b | 121 / 146 |
| pdfa-3u | 123 / 148 |
| pdfa-4 | 105 / 109 |
| pdfa-4e | 105 / 109 |
| pdfa-4f | 105 / 109 |

## Corpus

The Java-free parity corpus remains stable: 10 / 10 rows match, with 0 expected
drift and 0 unexpected drift. The generated resource row now requires
`resourceUse`, `font`, `colorSpace`, `outputIntent`, `extGState`, `image`, and
`formXObject` feature families.

`corpus-agreement.json` records `liveOracleEnabled: false`; live veraPDF oracle
execution remains part of the later oracle-automation workstream. This G4
snapshot is the bounded Java-free evidence for the resource/color/image/XObject
semantic slice.

## Remaining Readiness Work

The remaining release-blocking unsupported clusters are assigned to later
readiness workstreams: G5 XMP RDF metadata validation and G6 accessibility
reconstruction.
