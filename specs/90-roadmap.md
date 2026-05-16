# Roadmap — Incremental Delivery

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-16

## 0. Principles

- Always shippable: every milestone leaves standard quality gates green.
- Library before CLI polish: the CLI uses the public Rust API.
- Correct spine first: a small correct rule catalog beats a broad unreliable one.
- Hostile input always: resource limits and panic-free parsing are milestone criteria, not hardening extras.

## 1. Milestones

### M0 — First end-to-end validator

User-visible result: `pdfv validate sample.pdf --format json` parses a simple PDF, runs a small built-in profile/rule set, and emits JSON/text with deterministic exit codes.

Specs touched: 00, 10, 11, 12, 13, 20, 50, 61, 70, 72.

Exit criteria:

- `pdfv-core::Validator::validate_path` and `validate_reader` exist with documented examples.
- CLI validates one or more files and emits JSON/text.
- Parser handles header, indirect objects, dictionaries, arrays, classic xref table, trailer, basic streams, and parse facts.
- Rule engine supports enough IR for the M0 built-in rules and reports unsupported rules.
- No parser/validator panics on malformed fixture corpus.
- Standard verification commands pass.

### M1 — Modern PDF coverage

User-visible result: more real-world PDFs validate far enough to produce useful reports, including xref streams and FlateDecode streams.

Exit criteria:

- Xref stream and object stream support.
- FlateDecode with byte-counted streaming limits.
- Expanded COS/PD model wrappers for pages, metadata, output intents, fonts, annotations, and content streams.
- Fuzz targets run locally and first regression corpus lands.
- Performance budgets from [71-performance-budgets.md](./71-performance-budgets.md) measured.

### M2 — Profile breadth and custom profiles

User-visible result: users can validate against a broader PDF/A profile subset and load custom profile files.

Exit criteria:

- `spike-profile-expression-ir.md` complete and reflected in rule IR.
- Profile generator/importer handles representative veraPDF XML rules.
- Custom profile XML loading behind `custom-profiles`.
- Unsupported rules are reported with clear reasons.
- CLI `profiles list` implemented.

### M3 — Batch-grade CLI and CI integration

User-visible result: `pdfv` is practical for document repositories and CI jobs.

Exit criteria:

- Recursive discovery, bounded `--jobs`, YAML config, output files, redacted paths.
- Batch reports and exit-code aggregation stable.
- JSON schema examples documented.
- Benchmarks and audit/deny gates documented.

### M4 — Compatibility and advanced validation

User-visible result: compatibility features are available for organizations comparing against existing veraPDF workflows.

Exit criteria:

- Decision on MRR/XML compatibility from `spike-mrr-compatibility.md`.
- XML compatibility output available through `pdfv validate --format xml`; `mrr` accepted only as a deprecated alias.
- Larger profile coverage and richer feature facts for page, font, annotation, output-intent, and content-stream objects.
- Password/decryption support scoped by [14-password-decryption-design.md](./14-password-decryption-design.md): Standard security handler revisions 2-4 in the first implementation phase; revisions 5-6 after the AES-256 risk gate.
- Public conformance fixture matrix published.

### M5 — Parser and profile parity foundation

User-visible result: more real-world PDFs reach deterministic validation because stream filters, xref chains, and built-in profile selection match the major veraPDF surfaces.

Exit criteria:

- Stream decoders cover Flate predictors, ASCIIHex, ASCII85, LZW, RunLength, and Crypt identity/named filters as scoped by [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md).
- Large-file source storage avoids eager whole-file memory residency above a configurable threshold.
- Built-in profile catalog lists PDF/A-1/2/3/4, PDF/UA, and WTPDF profiles from the vendored XML set.
- Generated profile coverage reports executable and unsupported rule counts per profile.
- Auto profile selection no longer silently treats absent metadata as detected PDF/A-1B.

### M6 — Validation model and metadata parity

User-visible result: official profile rules evaluate against a broad validation model rather than the current small fact subset.

Exit criteria:

- Validation model families cover document/catalog, page/resources, fonts/CMaps, images/content, annotations/actions/forms, color/transparency, structure/accessibility, and signature/security facts.
- Generated built-in profile rules are schema-checked against model properties and links.
- XMP metadata parsing detects PDF/A, PDF/UA, and WTPDF flavour claims with structured report evidence.
- Unsupported official rules are visible in reports with profile/rule citations and do not produce compliant status.

### M7 — veraPDF product-surface parity

User-visible result: migration workflows can use pdfv for validation reports, feature inventory, policy checks, and safe metadata repair where explicitly supported.

Exit criteria:

- Feature extraction emits bounded JSON/XML feature reports for the model families used by policies.
- Policy reports consume feature reports and merge into validation output.
- Metadata repair is available as an explicit non-in-place command with atomic output writes and refusal reports.
- Raw XML and static HTML report formats are available.
- CLI compatibility documentation lists supported, intentionally different, and out-of-scope veraPDF flags.

## 2. Calendar estimate

For one focused developer:

- M0: 3-5 weeks.
- M1: 4-6 weeks.
- M2: 4-8 weeks, driven by profile expression coverage.
- M3: 2-4 weeks.
- M4: research-dependent; password/decryption adds approximately 2-4 weeks for revisions 2-4, a separate AES-256 risk gate, and approximately 1 week for the dedicated revisions 5-6 implementation phase.
- M5: 4-8 weeks, driven by decoder coverage and profile-generator coverage.
- M6: 8-16 weeks, driven by validation-model breadth and XMP semantics.
- M7: 6-12 weeks, depending on policy language and metadata repair scope.

## 3. Cross-references

- Engineer order: [91-impl-plan.md](./91-impl-plan.md)
- Load-bearing choices: [99-key-decisions.md](./99-key-decisions.md)
