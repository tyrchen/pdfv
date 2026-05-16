# Key Decisions

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-15

## D1 — Build library-first, CLI second

- Context: public architecture.
- Alternatives considered: CLI-only binary; parser crate first with no validation API; library-first validator.
- Decision: `pdfv-core` owns validation APIs and `pdfv` CLI is an adapter.
- Why: veraPDF’s `ProcessorFactory`/`ProcessorImpl` split keeps CLI config separate from validation execution (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorFactory.java:95`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorImpl.java:103`), and Rust consumers need embeddability.
- Pinned by: [00-prd.md](./00-prd.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [50-cli-design.md](./50-cli-design.md)
- Date: 2026-05-15

## D2 — Tolerant parser facts are validation inputs

- Context: parser/validator boundary.
- Alternatives considered: strict parse-or-fail; warning logs only; structured parse facts.
- Decision: parser anomalies become structured `ParseFact` values available to rules and reports.
- Why: PDF conformance often depends on malformed-but-parseable details such as stream length and EOL spacing; veraPDF records these facts during parsing (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:190`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:232`).
- Pinned by: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md)
- Date: 2026-05-15

## D3 — Replace `ThreadLocal` state with explicit `ValidationSession`

- Context: mutable validation/parser caches.
- Alternatives considered: global state; thread-local state; explicit per-input session.
- Decision: every validation run owns a `ValidationSession` containing caches, variables, traversal state, and limits.
- Why: veraPDF uses `ThreadLocal` document/flavour/cache state (`vendors/veraPDF-parser/src/main/java/org/verapdf/tools/StaticResources.java:50`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/containers/StaticContainers.java:37`), which is avoidable in Rust and would complicate Send/Sync guarantees.
- Pinned by: [13-validation-engine-design.md](./13-validation-engine-design.md), [70-security.md](./70-security.md)
- Date: 2026-05-15

## D4 — Use bounded Rust rule IR, not JavaScript

- Context: profile rule execution.
- Alternatives considered: embed JavaScript engine; hand-code every rule; convert profile rules into bounded IR.
- Decision: profiles compile to a deterministic, side-effect-free `RuleExpr` IR with instruction/depth budgets.
- Why: veraPDF’s JavaScript evaluator is flexible but not a good default for a hostile-input Rust library. A bounded IR is safer, testable, and easier to fuzz.
- Pinned by: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [70-security.md](./70-security.md)
- Date: 2026-05-15

## D5 — M0 is end-to-end with narrow rule breadth

- Context: delivery plan.
- Alternatives considered: parser-only M0; broad profile M0; narrow end-to-end M0.
- Decision: M0 includes parser, profile subset, validation engine, report writers, and CLI, but only a small built-in rule catalog.
- Why: parser-only would not validate the public architecture; broad profile coverage would stall on rule conversion before users can try the tool.
- Pinned by: [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- Date: 2026-05-15

## D6 — JSON/text first, MRR later

- Context: report formats.
- Alternatives considered: implement MRR/XML immediately; JSON/text first; text-only.
- Decision: M0 stabilizes JSON and text. MRR/XML requires `spike-mrr-compatibility.md`.
- Why: JSON is the natural Rust/CI integration format; MRR compatibility is valuable but should not block the validator spine.
- Pinned by: [20-reporting-design.md](./20-reporting-design.md), [90-roadmap.md](./90-roadmap.md)
- Date: 2026-05-15

## D7 — Remove the server placeholder until explicitly scoped

- Context: workspace application shape.
- Alternatives considered: keep `apps/server` as a placeholder; rename it to `apps/cli`; keep both apps.
- Decision: remove the server placeholder and add `apps/cli` as the M0 application crate.
- Why: the roadmap and implementation plan define an embeddable library plus CLI. A network service has no product, security, or API spec yet, and keeping a placeholder crate makes quality gates cover code outside the scoped product.
- Pinned by: [61-crates-and-features.md](./61-crates-and-features.md), [91-impl-plan.md](./91-impl-plan.md)
- Date: 2026-05-15

## D8 — Canonicalize machine-readable compatibility output as XML

- Context: M4 report compatibility.
- Alternatives considered: keep JSON/text only; implement `mrr` as the primary format name; implement `xml` with `mrr` as an alias; implement veraPDF raw XML/HTML/report bundles.
- Decision: add `ReportFormat::Xml`, expose `pdfv validate --format xml`, and accept `--format mrr` only as a deprecated alias to the same writer.
- Why: current veraPDF documentation says `xml` and `mrr` refer to the same report format and `mrr` is deprecated starting with veraPDF 1.24. The product should match current terminology while keeping migration scripts easy to adapt.
- Pinned by: [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md), [90-roadmap.md](./90-roadmap.md), [../docs/research/spike-mrr-compatibility.md](../docs/research/spike-mrr-compatibility.md)
- Date: 2026-05-16
