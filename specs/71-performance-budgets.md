# 71-performance-budgets: Performance and Resource Budgets

Status: draft · Owner: pdfv · Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md)

## 1. Purpose

Performance work must preserve safety and correctness. This spec defines budgets and measurement gates so parser and validation changes do not regress silently.

## 2. M0 budgets

Measured on a current Apple Silicon developer laptop:

| Scenario | Budget |
| --- | --- |
| 10 MB simple non-encrypted PDF, JSON report | p50 under 2 s, peak RSS under 256 MB |
| 10 MB supported encrypted PDF, correct password, JSON report | p50 under 3 s, peak RSS under 320 MB |
| 100 small PDFs in batch, 4 jobs | p50 per file under 250 ms after warmup |
| malformed random 1 MB input | fail/report under 500 ms |
| stream declared length mismatch with scan fallback | bounded by configured scan cap |
| content stream with 10k simple operators | p50 under 1 s, peak additional RSS under 64 MB |
| 1k-page resource inheritance traversal | p50 under 2 s, no unbounded per-page clone of inherited resources |
| 10k-node structure tree traversal | p50 under 2 s, exits with cap warning before hard memory growth |

These are starting budgets, not claims of final competitiveness.

## 3. Allocation rules

- Parser tokenization works on bytes and borrows where possible.
- Stream decoding is lazy and bounded.
- Decryption is streaming for streams and bounded for strings; decrypted bytes count against explicit decrypted-byte resource limits before downstream decoding.
- Rule lookup uses precomputed maps by object type.
- Validation traversal uses `Vec` stacks with capacity hints.
- Content-stream operator summaries are built lazily and capped by `max_content_stream_ops`.
- Effective resource contexts share inherited summaries by session-local cache, not by cloning entire resource maps per page.
- Accessibility graph reconstruction is lazy and capped by node/depth/parent-tree limits.
- Report writers stream to `Write` and avoid duplicate full-report strings.
- Use `SmallVec` only after profiling shows repeated small-vector pressure.

## 4. Bench harness

Criterion benchmarks land after Phase 2 when parser shapes stabilize. Bench data includes fixture name, file size, object count, stream count, checks executed, elapsed time, and peak memory when available. Benchmarks are not unit tests and can be ignored locally unless running perf gates.

M6-M7 parity phases add targeted benches for content streams, effective resources, font/color summaries, and accessibility graph traversal. These benches must record operator count, page count, resource count, structure node count, and bound-rule count where applicable.

## 5. Regression policy

Any change that worsens a tracked benchmark by more than 10 percent must either be justified by correctness/security gains in the PR text or fixed before merge.

## 6. Cross-references

- ← Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md)
- → Constrains: [91-impl-plan.md](./91-impl-plan.md)
