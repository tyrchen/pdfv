# M9 G1 Expression Coverage Closure Snapshot

Status: Done · Owner: pdfv · Date: 2026-05-17

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g1-expression-coverage-closure/`](./m9-g1-expression-coverage-closure/) for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G1 Summary

| Metric | Count |
| --- | ---: |
| Imported official rules | 6,749 |
| Unsupported-expression rules | 0 |
| Unsupported-expression clusters | 0 |
| Remaining unsupported rules | 1,602 |
| Remaining unsupported clusters | 239 |
| Release-blocking unsupported rules | 1,364 |
| Release-blocking unsupported clusters | 196 |

Unsupported expressions are now below the G1 threshold of 1% of imported
in-scope rules. The current rate is 0%.

## Remaining Classification

All remaining unsupported rules have review-backed non-expression reasons:

| Primary reason | Rules |
| --- | ---: |
| missingProperty | 1,560 |
| missingSemanticFamily | 42 |

These are owned by later readiness phases: derived properties in G2,
font/CMap semantics in G3, resource/color/image semantics in G4, XMP RDF facts
in G5, and accessibility reconstruction in G6.

## Regression Coverage

G1 added unit coverage for:

- XML entity-split rule text, including `&lt;`, `&gt;`, and `&amp;&amp;`.
- Official arithmetic, modulo, exponent, comparison, and ternary expressions.
- Flag-mask and shift fragments such as `(F & 512) == 512`.
- Method-style predicates: `contains`, `length`, `search`, `indexOf`.
- Collection predicates: `split`, `filter`, `slice`, and `findIndex`.
- `Math.abs` width-difference checks.

The generated-profile regression asserts every vendored profile has
`unsupportedExpression == 0`.
