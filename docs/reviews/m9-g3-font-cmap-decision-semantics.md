# M9 G3 Font and CMap Decision Semantics Snapshot

Status: Done | Owner: pdfv | Date: 2026-05-17

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g3-font-cmap-decision-semantics/`](./m9-g3-font-cmap-decision-semantics/)
for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G3 Summary

| Metric | Count |
| --- | ---: |
| Imported official rules | 6,749 |
| Remaining unsupported rules | 752 |
| Remaining unsupported clusters | 97 |
| Release-blocking unsupported rules | 605 |
| Release-blocking unsupported clusters | 87 |
| G3 font/CMap clusters | 0 |
| G3 release-blocking clusters | 0 |

G3 removed every missing-property cluster assigned to font, CMap, and embedded
font-file decision semantics. The implemented facts are bounded summaries:
font subtype/name, Standard 14 detection, descriptor flags, embedded program
presence, font-file subtype, ToUnicode, CMap identity/WMode/CIDSystemInfo,
CIDSet/CharSet presence, glyph evidence, and shallow embedded font-file
validity.

## Bound Coverage

PDF/A profile bound-rule coverage improved again after the G2 snapshot:

| Flavour | Bound / total |
| --- | ---: |
| pdfa-1a | 85 / 135 |
| pdfa-1b | 79 / 129 |
| pdfa-2a | 101 / 153 |
| pdfa-2b | 92 / 144 |
| pdfa-2u | 94 / 146 |
| pdfa-3a | 103 / 155 |
| pdfa-3b | 94 / 146 |
| pdfa-3u | 96 / 148 |
| pdfa-4 | 78 / 109 |
| pdfa-4e | 78 / 109 |
| pdfa-4f | 78 / 109 |

## Corpus

The Java-free parity corpus remains stable: 9 / 9 rows match, with 0 expected
drift and 0 unexpected drift.

## Remaining Readiness Work

The remaining release-blocking unsupported clusters are assigned to later
readiness workstreams: G4 resource/color/image/XObject semantics, G5 XMP RDF
metadata validation, G6 accessibility reconstruction, and G7 oracle drift
triage.
