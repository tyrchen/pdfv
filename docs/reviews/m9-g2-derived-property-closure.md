# M9 G2 Derived Property Closure Snapshot

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
[`m9-g2-derived-property-closure/`](./m9-g2-derived-property-closure/) for
this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G2 Summary

| Metric | Count |
| --- | ---: |
| Imported official rules | 6,749 |
| Remaining unsupported rules | 1,037 |
| Remaining unsupported clusters | 130 |
| Release-blocking unsupported rules | 861 |
| Release-blocking unsupported clusters | 111 |
| G2 missing-property clusters | 0 |
| G2 release-blocking clusters | 0 |

G2 removed every missing-property cluster assigned to derived property closure.
The remaining missing-property backlog is assigned to later readiness phases:

| Phase | Rules |
| --- | ---: |
| G3 font/CMap semantics | 285 |
| G4 resource/color/image/XObject semantics | 272 |
| G5 XMP RDF facts | 231 |
| G6 accessibility semantics | 207 |

## Bound Coverage

PDF/A profile bound-rule coverage now ranges from 48% to 55%. No PDF/A profile
has an unassigned G2 blocker left; the sub-60% profiles are blocked by the
scheduled G3/G4/G5/G6 semantic clusters.

| Flavour | Bound / total |
| --- | ---: |
| pdfa-1a | 66 / 135 |
| pdfa-1b | 61 / 129 |
| pdfa-2a | 77 / 153 |
| pdfa-2b | 71 / 144 |
| pdfa-2u | 71 / 146 |
| pdfa-3a | 80 / 155 |
| pdfa-3b | 74 / 146 |
| pdfa-3u | 74 / 148 |
| pdfa-4 | 59 / 109 |
| pdfa-4e | 60 / 109 |
| pdfa-4f | 60 / 109 |

## Source Evidence

The model schema report now records property source-evidence counts. The G2
families include:

| Family | Properties | Evidence |
| --- | ---: | --- |
| object | 25 | semanticGraph: 25 |
| document | 43 | semanticGraph: 41, xmpRdf: 2 |
| metadata | 17 | directCos: 4, semanticGraph: 2, xmpRdf: 11 |
| annotation | 37 | directCos: 11, semanticGraph: 26 |
| image | 14 | directCos: 11, semanticGraph: 3 |
| outputIntent | 14 | decodedStream: 8, directCos: 5, semanticGraph: 1 |

## Corpus

The Java-free parity corpus remains stable: 9 / 9 rows match, with 0 expected
drift and 0 unexpected drift.
