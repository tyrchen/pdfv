# PRD — pdfv Rust PDF Validator

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-15

## 1. Problem

Rust programs need a PDF validation library that is embeddable, safe on hostile files, and ergonomic enough for CI, ingestion pipelines, and command-line use. Existing high-quality validators such as veraPDF prove the value of profile-driven PDF/A validation, tolerant parsing, and rich reports, but their Java architecture brings runtime global registration, thread-local validation state, JavaScript expression execution, and JVM deployment constraints. The Rust ecosystem needs the same validator shape with Rust-native APIs, bounded resource controls, structured errors, and a CLI that is useful from day one.

## 2. Vision

`pdfv` is a Rust-native validation toolkit: `pdfv-core` exposes a stable library API, `pdfv-cli` provides a fast command-line interface, and validation remains data-driven through profiles rather than hard-coded ad hoc checks. The first complete user slice validates one PDF, emits JSON/text, preserves parse anomalies as rule-queryable facts, and never panics on malformed input.

```rust
use pdfv_core::{FlavourSelection, ValidationOptions, Validator};

let validator = Validator::new(ValidationOptions::builder()
    .flavour(FlavourSelection::Auto { default: None })
    .max_failed_assertions_per_rule(100)
    .build())?;
let report = validator.validate_path("invoice.pdf")?;
assert!(report.is_validated());
```

```bash
pdfv validate invoice.pdf --flavour auto --format json
```

## 3. Goals

| # | Goal | Measure |
| --- | --- | --- |
| G1 | Ship a reusable Rust library API before CLI-only behaviour. | `pdfv-core` can validate from `Path` and `Read + Seek`; CLI uses only public library APIs. |
| G2 | Preserve veraPDF’s load-bearing validation model. | Parser facts, profile rules, object graph traversal, deferred rules, bounded failed assertion reporting, and flavour selection are implemented per [10-data-model.md](./10-data-model.md). |
| G3 | Treat malicious PDFs as expected input. | No `unsafe`, `unwrap`, `expect`, panics, unchecked arithmetic, unbounded decompression, or recursion on external data in parser/validator modules. |
| G4 | Provide useful CLI output for automation. | `pdfv validate --format json` is stable, documented, and returns deterministic exit codes for valid, invalid, parse failure, usage failure, and internal failure. |
| G5 | Make conformance growth incremental. | New profile rules can be added without changing parser public APIs; unsupported rules are reported as structured `RuleUnsupported`, not silently skipped. |
| G6 | Keep performance predictable. | M0 validates a 10 MB non-encrypted PDF fixture under 2 seconds on a current Apple Silicon developer laptop, with peak RSS under 256 MB. |

## 4. Non-goals

- M0 is not a complete PDF/A conformance clone of veraPDF. It is a correct spine with a deliberately small rule catalog.
- M0 does not repair metadata, extract full feature reports, render PDFs, modify PDFs, or validate digital signatures cryptographically.
- M0 does not provide MRR/XML compatibility. JSON/text are the first stable outputs; MRR is a later compatibility milestone.
- M0 does not embed a JavaScript engine for profile expressions.
- M0 does not expose a network service.

## 5. Users

- Primary: Rust application developers validating uploaded or ingested PDFs inside a service, batch job, or desktop app.
- Primary: CI/release engineers who need a CLI to gate documents or archives.
- Secondary: PDF tooling maintainers who need parse facts and validation diagnostics for debugging.
- Anti-persona: users looking for PDF rendering, editing, OCR, accessibility remediation, or a complete legal certification workflow in the first release.

## 6. Success metrics

- A new developer can run `cargo run -p pdfv-cli -- validate samples/minimal.pdf --format json` and receive a valid structured report in under 60 seconds from checkout.
- Public API docs include examples for `validate_path`, `validate_reader`, and report serialization, and doc tests pass.
- Fuzzing finds no parser panic on malformed byte input for the parser entrypoint corpus after a 10 minute local run.
- CI gates include build, tests, nightly fmt, pedantic clippy, audit, deny, and targeted parser safety lints.

## 7. Naming conventions

- Workspace crates use the `pdfv-*` prefix: `pdfv-core`, `pdfv-cli`, later `pdfv-profiles` if generated profile assets need separation.
- Public JSON fields use `camelCase` per AGENTS.md Serialization & Data.
- Public Rust types use PDF terms only when they match PDF specification usage: `CosObject`, `PdfDocument`, `ValidationProfile`, `RuleId`, `Assertion`, `ParseFact`.
- The CLI binary is `pdfv`; the primary command is `pdfv validate`.

## 8. Research basis

This PRD binds the findings in [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md): copy veraPDF’s library-first processor split, profile-driven validation, tolerant parser facts, iterative object graph traversal, bounded report shape, and CLI-as-config-adapter; avoid `ThreadLocal` session state, JavaScript rule execution, recover-by-logging only, and metadata fixing in the initial validator core.

