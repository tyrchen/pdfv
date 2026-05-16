# 71-performance-budgets: Performance and Resource Budgets

Status: draft · Owner: pdfv · Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md)

## 1. Purpose

Performance work must preserve safety and correctness. This spec defines budgets and measurement gates so parser and validation changes do not regress silently.

## 2. M0 budgets

Measured on a current Apple Silicon developer laptop:

| Scenario | Budget |
| --- | --- |
| 10 MB simple non-encrypted PDF, JSON report | p50 under 2 s, peak RSS under 256 MB |
| 100 small PDFs in batch, 4 jobs | p50 per file under 250 ms after warmup |
| malformed random 1 MB input | fail/report under 500 ms |
| stream declared length mismatch with scan fallback | bounded by configured scan cap |

These are starting budgets, not claims of final competitiveness.

## 3. Allocation rules

- Parser tokenization works on bytes and borrows where possible.
- Stream decoding is lazy and bounded.
- Rule lookup uses precomputed maps by object type.
- Validation traversal uses `Vec` stacks with capacity hints.
- Report writers stream to `Write` and avoid duplicate full-report strings.
- Use `SmallVec` only after profiling shows repeated small-vector pressure.

## 4. Bench harness

Criterion benchmarks land after Phase 2 when parser shapes stabilize. Bench data includes fixture name, file size, object count, stream count, checks executed, elapsed time, and peak memory when available. Benchmarks are not unit tests and can be ignored locally unless running perf gates.

## 5. Regression policy

Any change that worsens a tracked benchmark by more than 10 percent must either be justified by correctness/security gains in the PR text or fixed before merge.

## 6. Cross-references

- ← Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md)
- → Constrains: [91-impl-plan.md](./91-impl-plan.md)

