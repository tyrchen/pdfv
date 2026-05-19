# Key Decisions

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17

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

## D9 — Scope password support to explicit Standard security handler phases

- Context: M4 encrypted PDF support.
- Alternatives considered: keep encrypted PDFs unsupported; implement all PDF encryption revisions at once; support Standard security handler revisions 2-4 first and defer revisions 5-6 behind a risk gate; accept literal CLI passwords for veraPDF-like convenience.
- Decision: Phase 9 supports password-protected PDFs only for `/Filter /Standard` revisions 2-4, using redacted password sources and no literal `--password <text>` CLI argument. Revisions 5-6 require a separate AES-256 risk gate before implementation.
- Why: revisions 2-4 unlock common older encrypted PDFs and align with the current parser/object model. Revisions 5-6 use a different key retrieval/hash path and `/OE`/`/UE`/`/Perms` handling (`vendors/veraPDF-parser/src/main/java/org/verapdf/tools/EncryptionToolsRevision5_6.java:40`, `vendors/veraPDF-parser/src/main/java/org/verapdf/tools/EncryptionToolsRevision5_6.java:96`), so bundling them into the first password phase would hide risk. Literal CLI passwords violate AGENTS.md secret-handling expectations.
- Pinned by: [14-password-decryption-design.md](./14-password-decryption-design.md), [50-cli-design.md](./50-cli-design.md), [70-security.md](./70-security.md), [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- Date: 2026-05-16

## D10 — Clear AES-256 decryption for a dedicated implementation phase

- Context: Phase 10 AES-256 decryption risk gate.
- Alternatives considered: keep Standard security handler revisions 5-6 explicitly unsupported; implement revisions 5-6 immediately in Phase 10; clear an implementation-ready design and fixtures for a later phase.
- Decision: Standard security handler revisions 5-6 are implementation-ready but remain a dedicated future implementation phase. The public password API, CLI secret-source policy, encrypted status, and redacted reporting contracts stay unchanged.
- Why: revision 5-6 support requires Algorithm 2.A/2.B key retrieval, UTF-8 password truncation to 127 bytes, `/OE`/`/UE`, AESV3 content decryption, direct file-key object decryption, and `/Perms` validation. These fit the current parser/decryption architecture, but they are a separate risk surface from the already-landed revisions 2-4 compatibility layer.
- Pinned by: [14-password-decryption-design.md](./14-password-decryption-design.md), [70-security.md](./70-security.md), [91-impl-plan.md](./91-impl-plan.md), [../docs/research/spike-aes-256-decryption.md](../docs/research/spike-aes-256-decryption.md)
- Date: 2026-05-16

## D11 — Treat veraPDF parity as layered compatibility, not a single rewrite

- Context: post-M4 gap analysis against vendored veraPDF code.
- Alternatives considered: declare M4 complete as the final scope; attempt one broad "full parity" phase; split parity into parser/profile/model/metadata/product layers.
- Decision: add explicit parity specs and phases for parser/filter/source parity, profile catalog/rule parity, validation model breadth, XMP/flavour detection, and optional product surfaces.
- Why: veraPDF spans parser filters, all official profile XMLs, hundreds of validation-model wrappers, XMP flavour detection, feature extraction, metadata repair, policy reporting, and multiple report formats. A single phase would hide dependency risk and make review impossible. Layered parity keeps the Rust safety model intact while making each gap measurable.
- Pinned by: [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md), [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- Date: 2026-05-16

## D12 — Full profile parity uses generated bounded Rust data, not runtime JavaScript

- Context: official veraPDF profile compatibility.
- Alternatives considered: keep only `pdfv-m4`; embed a JavaScript engine; hand-code profile rules; generate bounded Rust profile data from vendored XML.
- Decision: built-in profile parity is generated from vendored XML into deterministic Rust data, with unsupported expressions retained as structured report data until the bounded IR supports them.
- Why: veraPDF evaluates JavaScript-style profile expressions, but runtime JavaScript conflicts with the hostile-input Rust security model. Generation gives repeatable coverage metrics, reviewable diffs, and profile/rule citations while preserving [D4](./99-key-decisions.md#d4--use-bounded-rust-rule-ir-not-javascript).
- Pinned by: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [70-security.md](./70-security.md)
- Date: 2026-05-16

## D13 — Product-surface parity remains opt-in and read/write separated

- Context: feature extraction, policy reports, and metadata repair.
- Alternatives considered: make validation always extract all features; mix metadata repair into `validate`; keep product surfaces out of scope forever.
- Decision: feature extraction and policy reports are opt-in read-only validation-adjacent surfaces; metadata repair is a separate non-in-place command with atomic writes and refusal reports.
- Why: veraPDF supports validation, feature extraction, and metadata repair, but repair changes files and carries a different safety/security contract. Keeping read-only validation separate from write-capable repair preserves default safety while still providing migration paths.
- Pinned by: [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md), [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md), [70-security.md](./70-security.md)
- Date: 2026-05-16

## D14 — Split model parity into operator, resource, and accessibility subsystems

- Context: drift review after comparing `vendors` and `pdfv`.
- Alternatives considered: keep one broad validation-model phase; hand-code only the currently failing rules; split content streams, resource/font/color semantics, and accessibility into independent subsystems.
- Decision: the validation model registry remains the spine, but content-stream operator facts, resource/font/color semantics, and structure/accessibility reconstruction are separate specs and phases.
- Why: veraPDF's drift is concentrated in mature semantic layers, not the CLI. Content operators feed resource usage and accessibility marked-content association; resource/font/color semantics feed PDF/A rule coverage; accessibility reconstruction feeds PDF/UA/WTPDF rule coverage. A single broad phase would hide blockers and make parity metrics meaningless.
- Pinned by: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [91-impl-plan.md](./91-impl-plan.md), [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
- Date: 2026-05-16

## D15 — Treat parity as measured coverage, not asserted completeness

- Context: official profile XML volume is close between veraPDF and pdfv, but executable and bound rule coverage are not equivalent to XML line count.
- Alternatives considered: use LoC as the progress metric; use profile XML import counts only; require full live veraPDF report byte parity; track imported/lowered/bound rules, unsupported reasons, model families, feature families, and semantic corpus agreement.
- Decision: every parity milestone must emit deterministic coverage metrics with imported, executable, bound, unsupported, and corpus-agreement counts. Byte-identical veraPDF reports are not the general parity metric.
- Why: generated XML can make the catalog look complete while rules remain unsupported by missing expression features or model semantics. The project needs a reviewable signal that points engineers to the next missing object, property, link, or semantic family.
- Pinned by: [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [72-testing-strategy.md](./72-testing-strategy.md), [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md), [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
- Date: 2026-05-16

## D16 — Gate veraPDF-grade claims on oracle evidence and zero false-compliant drift

- Context: readiness to use pdfv for production PDF/A/PDF/UA validation on arbitrary real-world PDFs.
- Alternatives considered: claim readiness from aggregate bound-rule percentage; require 100% byte-identical veraPDF reports; require decision-grade oracle agreement plus unsupported-rule burn-down; defer any readiness claim indefinitely.
- Decision: a veraPDF-grade claim requires in-scope PDF/A/PDF/UA rule coverage thresholds, unsupported-rule cluster closure, live oracle corpus agreement, 100% mismatch classification, and zero false-compliant rows. Byte-identical report output is not required.
- Why: aggregate bound-rule counts can hide weak PDF/A coverage, while byte-identical report parity would overfit report formatting instead of validation decisions. The real production risk is returning `Valid` when veraPDF would reject or when required semantics are unsupported. Oracle agreement plus zero false-compliant drift targets that risk directly.
- Pinned by: [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md), [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md), [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md), [90-roadmap.md](./90-roadmap.md)
- Date: 2026-05-17

## D17 — Withhold the first veraPDF-grade claim until T3 release-lab evidence exists

- Context: M9 G8 release hardening regenerated profile, parity, unsupported-cluster, and public T2 oracle evidence, but no private T3 real-world corpus manifest was supplied in-repository.
- Alternatives considered: make the first claim from full rule binding plus public T2 rows; record a smaller T3 corpus threshold without data; withhold the claim until the T3 release-lab run exists.
- Decision: do not make the first public veraPDF-grade PDF/A/PDF/UA readiness claim from the G8 evidence package alone. The claim remains blocked until a release-lab T3 snapshot records the corpus size, stratification, outcome-class match rate, mismatch classification, false-compliant count, and any statistically justified corpus-size exception. The `make readiness-release-gate` target enforces the T3 blocker by requiring release-mode oracle summary checks.
- Why: the G8 snapshot is strong implementation evidence, but `docs/reviews/m9-g8-readiness-release-hardening/oracle-summary.json` contains only 3 public T2 rows and 0 T3 rows. Claiming arbitrary real-world readiness without T3 evidence would violate the oracle gate and weaken the project's false-compliant risk control.
- Pinned by: [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md), [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md), [../docs/verapdf-readiness.md](../docs/verapdf-readiness.md), [../docs/reviews/m9-g8-readiness-release-hardening.md](../docs/reviews/m9-g8-readiness-release-hardening.md)
- Date: 2026-05-19
