# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [pdfv-v0.1.2](https://github.com/compare/pdfv-v0.1.1..pdfv-v0.1.2) - 2026-11-23

### Miscellaneous Chores

- add specs - ([9d274b1](https://github.com/commit/9d274b1a05ece0db0311e706e2f82b4c30f4f973)) - Tyr Chen
- add new specs - ([b9785f6](https://github.com/commit/b9785f6c6a56c2e1c9a9b22e95e8d7510e60fc8c)) - Tyr Chen
- prep for publish - ([a604602](https://github.com/commit/a60460299b8ac51ad8ce96880df453ba1df1d116)) - Tyr Chen

### Other

- Update CHANGELOG.md - ([2b252c0](https://github.com/commit/2b252c0f6aac4f494f7daaba5430252d3caec9cb)) - Tyr Chen
- phase 14: bind model registry roots

Complete the phase 13 nested property path gap and land phase 14.1-14.2 registry/document-root work. Nested generated profile references now parse as property paths, schema checks walk registered links, validation materializes linked target families, and catalog/document roots expose deterministic links and registered catalog properties.

Exit evidence: workspace build, tests, fmt, clippy, docs, strict clippy, cargo audit, and cargo deny check pass. Existing non-code spec/doc edits in the worktree were left unstaged. - ([53da0d2](https://github.com/commit/53da0d256bf989494c802c7fd9e478d5d2bef63e)) - Tyr Chen
- phase 14: land page action form foundations

Implements Phase 14.3-14.5 model parity work from specs 17, 24, 70, and 72: page inheritance, annotation/action/form/filespec links, bounded cycle traversal coverage, model-schema parity metrics, and the Makefile parity-model-schema target.

Exit evidence: generated rules now report bound and unsupported model-schema counts, action/form/page traversal remains iterative under max_objects, and model_traversal_m6 bench is registered. Pre-existing spec/doc worktree changes are not included. - ([e25c017](https://github.com/commit/e25c017b1b929af85d0742b0f693b0fe2c1df250)) - Tyr Chen
- phase 14 review: fix model traversal parity gaps

Fixes the independent review findings for Phase 14.3-14.5: semantic-family unsupported grouping, name-tree Kids traversal with cycle caps, indirect additional actions, and array-backed destinations. - ([282f384](https://github.com/commit/282f3844b646acc4fb0fac732859fad8ca5616a5)) - Tyr Chen
- phase 15: add content stream operator model

Implements the M6 content stream operator phase across specs 15, 16, 17, 21, 24, 70, 71, and 72. Adds bounded decoded-stream tokenization, redacted operator summaries, resource-use facts for Tf/color/gs/sh/Do, lazy content stream summary caching, operator/marked-content/inline-image/resource-use model families, a Criterion content-stream bench, and a content-stream fuzz target.

Review fixed the in-phase unknown-operator binding gap by making unknown operator facts satisfy the undefinedOperator family. No deferred findings were added to specs/93-improvements-review.md. - ([b11e331](https://github.com/commit/b11e33111ef3cf8ce88c8abc6080fcb75b3f7ba1)) - Tyr Chen
- phase 16: add resource font color semantics

Land M6 resource semantics from specs 16, 17, 21, 22, 24, 70, 71, and 72: effective resource contexts with page inheritance and form XObject nesting, content-stream resource-use resolution statuses, shallow font/CMap/color/output-intent/ICC/XObject/pattern/shading/function schemas and summaries, session-bound resource caps, and a resource traversal benchmark.

Exit criteria met: resource uses resolve deterministically with missing and wrong-type facts; font/color/output-intent model families are registered and covered by fixtures; embedded font and ICC payloads remain capped and report-safe; standard gates, strict clippy, docs, parity schema, cargo audit, cargo deny, and resource traversal bench pass. No deferred findings. - ([2cb0d01](https://github.com/commit/2cb0d0194a442ffd3bc236cb8d07d62578a77ffe)) - Tyr Chen
- phase 17: add XMP flavour detection

Implement bounded XMP metadata extraction hardening, namespace-aware identification preservation, duplicate-claim facts, WTPDF multi-declaration profile mapping, and auto-selection coverage for phase 17 across specs 14, 16, 17, 18, 20, 70, and 72.

Quality gates passed before commit: cargo build --workspace --all-targets --all-features; cargo test --workspace --all-targets --all-features; cargo +nightly fmt -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; strict clippy with pedantic/unwrap/expect/indexing/panic lints; RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps; make parity-model-schema; cargo audit; cargo deny check with existing warnings only. - ([193fd3d](https://github.com/commit/193fd3d649cfd269adaa6fafb71acdb1c9fd53ca)) - Tyr Chen
- phase 17 review: fix XMP claim semantics

Fix the valid in-phase review findings: profile-specific PDF/A and PDF/UA identification model values, WTPDF declaration-context filtering, conflicting duplicate XMP claim facts, and XMP property/parse-fact retention caps.

Re-ran gates after the fixes: cargo build --workspace --all-targets --all-features; cargo test --workspace --all-targets --all-features; cargo +nightly fmt -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; strict clippy with pedantic/unwrap/expect/indexing/panic lints; RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps; make parity-model-schema; cargo audit; cargo deny check with existing warnings only. - ([1b595d7](https://github.com/commit/1b595d7178a8459114f20903f3b0a5514e8effd3)) - Tyr Chen
- phase 18: add accessibility graph semantics

Implements Phase 18 structure and accessibility semantics across specs 16, 17, 21, 22, 23, 24, 70, and 72. Adds bounded /StructTreeRoot traversal with role/class/ID/parent tree facts, MCID and marked-content association, accessibility semantic model families, report-safe redacted feature extraction, schema registration, fixtures, a fuzz target, and a 10k-node traversal bench.

Exit criteria evidence: PDF/UA/WTPDF structure-dependent schemas now bind to registered accessibility families or named missing semantic-family buckets; graph construction is lazy and bounded by structure/parent-tree limits; feature reports redact raw text; model-schema parity and the standard/strict quality gates pass. - ([e66028f](https://github.com/commit/e66028f70edef6c23e6ab01f209200b7109f027e)) - Tyr Chen
- phase 18 review: fix accessibility associations - ([c699055](https://github.com/commit/c699055ad451f01b08b459ef863da8d0fec7aa47)) - Tyr Chen
- phase 19: add deterministic parity artifacts

Adds Makefile parity targets for profile coverage, model schema, and Java-free corpus agreement. The reports emit deterministic profile-coverage, model-schema, unsupported-rule, and corpus-agreement JSON artifacts under target/parity with non-placeholder counts and one primary unsupported reason per unsupported rule.

Adds review-gated coverage decrease comparison against PDFV_PARITY_BASELINE_DIR plus milestone snapshot instructions. Corpus rows cover parser, profile, operator, resource/font/color, XMP, and accessibility families without requiring Java; live veraPDF oracle rows remain opt-in. - ([84974a5](https://github.com/commit/84974a534d28291bfd69d9a36b7130529a5b7b51)) - Tyr Chen
- phase 19 review: enforce parity gates

Fixes review findings by making unexpected corpus drift fail parity-corpus, requiring baseline artifacts and detecting removed profile/total/lowered/bound coverage decreases, and validating review notes against the affected profile, field, and counts instead of accepting generic docs text.

Adds normal CI execution for Java-free parity targets while keeping live veraPDF corpus tests opt-in. - ([8798338](https://github.com/commit/87983388a9f3439453c79eb7a611817d2c9a1a23)) - Tyr Chen
- phase 20: pin feature policy report contract

Add library-level coverage proving policy evaluation is driven by an extracted FeatureReport and that both feature and policy reports are exposed through the public validation facade. This supports Phase 20 specs 10, 19, 20, 50, 70, and 72 while preserving the existing read-only, bounded extraction implementation.

Update phase-numbered policy and CLI compatibility docs to match the current M8 Phase 20 scope. No deferred findings. - ([b13995b](https://github.com/commit/b13995b2e1c0dac1a7edb3a3f502b7817c11c3b7)) - Tyr Chen
- phase 20 review: fix policy feature selection

Ensure policy evaluation always runs on a FeatureReport that includes every policy rule family, even when the caller requested a narrow --extract family list. This prevents false policy outcomes from incomplete feature extraction and adds a regression test for the library facade.

Clarify that the CLI compatibility inventory includes Phase 21 preview surfaces while Phase 20 owns feature extraction and policy checks. - ([f94a91c](https://github.com/commit/f94a91c728ce270203a2ca66ed9398929a12f9f5)) - Tyr Chen
- phase 21: finalize metadata repair parity

Tighten the Phase 21 repair write path to use no-clobber atomic persistence, preserving the non-in-place and pre-existing-output safety contract from specs 19, 50, 70, and 72. Add regression coverage for the race-sensitive atomic copy behavior and update the veraPDF CLI compatibility note so raw/html reports and conservative metadata repair are documented as Phase 21 surfaces.

Exit criteria covered by repair/raw/html integration tests, core golden report tests, compatibility documentation, and the standard workspace gates. - ([1f24150](https://github.com/commit/1f2415079849b122bed41dab049a44f86509b535)) - Tyr Chen
- phase 21 review: harden metadata repair safety

Fix the independent review findings for Phase 21. Metadata repair now copies the input into a bounded temporary candidate, validates that exact candidate, and only then persists it with no-clobber semantics. Repair refuses any encrypted input indicated by parse facts, including successfully decrypted inputs, and late no-clobber conflicts are reported as invalid output-path refusals instead of internal failures.

Adds regression coverage for decrypted encrypted repair refusal and explicit rejection of literal --password without echoing the supplied secret. Full workspace gates pass after the fix. - ([3b8cede](https://github.com/commit/3b8cede9965d105350497e7aff0f45159435872c)) - Tyr Chen
- phase 22: add unsupported-rule burn-down baseline

Lands the M9 G0 readiness slice from specs 25, 26, and 28: deterministic unsupported-rule clusters, rule burn-down output with baseline deltas, Makefile targets, and a copied docs/reviews milestone snapshot.

Exit evidence: parity targets generate profile/model/corpus/unsupported/burn-down artifacts; baseline reports 1,711 unsupported rules across 232 clusters, with 1,401 release-blocking rules across 187 PDF/A-PDF/UA clusters. No deferred findings. - ([9e04d28](https://github.com/commit/9e04d28ed1614ace5cfc36f4790950959349eb6a)) - Tyr Chen
- phase 22: close expression coverage

Land M9 G1 expression coverage closure from specs 12, 16, 26, and 28. The veraPDF profile importer now preserves XML entity-split test text and lowers official comparison, arithmetic, modulo, flag-mask, ternary, regex, method, and collection predicate fragments into the bounded Rust IR.

The G1 snapshot records 6,749 imported official rules with 0 unsupportedExpression rules and 0 unsupportedExpression clusters. Remaining unsupported rules are reclassified as missingProperty or missingSemanticFamily for later readiness phases; standard gates, strict clippy, docs, audit, deny, and parity reports pass. - ([6fad4c9](https://github.com/commit/6fad4c9e5ae427c94f00abcc54d42dd84f8d2010)) - Tyr Chen
- phase 22: close derived property coverage

Implements G2 derived-property closure for the veraPDF parity track: model schema source-evidence counts, typed runtime support for document/page/annotation/image/outputIntent and shared object-derived properties, plus corrected burn-down phase ownership for remaining content-stream, XMP, and accessibility semantic gaps.

Exit evidence: regenerated profile/model-schema/corpus/unsupported-rule snapshots under docs/reviews/m9-g2-derived-property-closure, zero expectedPhase G2 clusters, zero G2 missing-property clusters, and corpus parity remains 9/9 with no drift. Review findings were fixed by removing compliant-looking document XMP placeholders from G2 and adding object runtime resolver/test coverage. - ([7dd8617](https://github.com/commit/7dd861779391256540976021e7e7325356a5b47b)) - Tyr Chen
- phase 22: close font cmap readiness clusters

Land M9 G3 font and CMap decision semantics from specs 26 and 28. The model registry now binds font, CMap, embedded-font-file, descriptor, glyph-evidence, ToUnicode, CIDSystemInfo, CMap WMode, and shallow embedded program facts needed to close the G3 unsupported-rule clusters.

The G3 milestone snapshot is published under docs/reviews/m9-g3-font-cmap-decision-semantics with parity profile, model-schema, corpus, unsupported-rule, cluster, and burn-down evidence. Remaining readiness work stays assigned to G4-G7. - ([12719ef](https://github.com/commit/12719ef05a2692f9dd20b91c22f71419529b74d9)) - Tyr Chen
- phase 22 review: prevent false-compliant font gaps

Expose raw subset font names, route unimplemented font/CMap byte-level checks to runtime Incomplete, and add a subset-font corpus row covering the corrected G3 behavior. - ([db46d73](https://github.com/commit/db46d73122f0817b3f3bdc0a2f61fcd195137720)) - Tyr Chen
- phase 22: close resource color image xobject readiness

Implements the G4 resource/color/image/XObject/output-intent semantic slice from specs 22/26/28, including color-space name and array resource materialization, output profile identity checks, bounded ICC output color facts, ExtGState/image/XObject properties, and generated corpus coverage.

G4 unsupported-rule clusters are now zero and the Java-free parity corpus remains 10/10. Live veraPDF oracle execution remains part of the later oracle-automation workstream. - ([35bcea0](https://github.com/commit/35bcea03c59f550fae76ed818edf2ac5546fa3df)) - Tyr Chen
- phase 22: close xmp rdf metadata validation readiness

Land G5 XMP RDF metadata validation from specs 18, 26, and 28: bounded RDF property, array, qualifier, packet-header, and encoding facts; metadata/document XMP rule bindings scoped to source streams; namespace-aware extension-schema checks; and refreshed G5 parity evidence.

G5 metadata clusters are zero and Java-free XMP corpus rows have no unexpected drift. Live veraPDF oracle remains disabled for this Java-free snapshot; remaining release-blocking clusters are G6 accessibility/readiness work. - ([b776c37](https://github.com/commit/b776c372273eeae6c0ff77d0ffad29a0860dcbf0)) - Tyr Chen
- phase 22: close pdf ua accessibility readiness

Land M9 G6 PDF/UA accessibility reconstruction by binding remaining structure-root, structure-element, annotation, table, list, heading, note, signature, and semantic guard properties required by the generated PDF/UA and WTPDF profiles. The accessibility graph keeps bounded parent-tree/class/attribute facts and report-safe structural strings while preserving raw text redaction.

Exit evidence is captured under docs/reviews/m9-g6-pdf-ua-accessibility-reconstruction: all unsupported-rule clusters are zero, PDF/UA and WTPDF profiles are fully bound, and the Java-free corpus remains 10/10 matches with no expected or unexpected drift. Live veraPDF oracle execution remains disabled for this snapshot and moves to G7. - ([55d6a07](https://github.com/commit/55d6a07e4be52d47c08f17ad14969b7d8c059013)) - Tyr Chen
- phase 22 review: harden accessibility semantics

Replace G6 pass-shaped accessibility sentinels with graph-derived namespace, table/header, note, and annotation target facts. Expand the Java-free corpus with explicit PDF/UA and WTPDF accessibility rows plus malformed table/header evidence.

Gates: cargo build --workspace --all-targets; cargo test --workspace --all-targets; cargo +nightly fmt -- --check; both clippy passes including pedantic unwrap/expect/indexing/panic; RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps; cargo audit; cargo deny check (existing warnings only). - ([ebeb4b5](https://github.com/commit/ebeb4b53cbefb11ef8fe404e2873aeb6321cedc0)) - Tyr Chen
- phase 22: add release oracle drift triage

Implements Phase 22 G7 release-oracle execution from specs 25, 27, and 28: Makefile-discoverable oracle-corpus and oracle-corpus-summary targets, typed oracle manifests/reports, T2/T3 manifests, live veraPDF comparison, mismatch classification, false-compliant gating, and the G7 milestone snapshot.

Exit evidence: Java-free parity remains 13/13 with 0 unexpected drift; live veraPDF 1.31.65 T2 oracle run produced 3/3 classified rows, 0 unexpected drift, 0 false-compliant rows, and 0 timeouts. T3 private corpus rows are supplied by release-lab manifest because real-world PDFs are not committed. - ([72a1361](https://github.com/commit/72a13619c3c08a1c823ef8aed443c67563f61f7e)) - Tyr Chen
- phase 22 review: require explicit oracle drift triage

Fixes the G7 review finding that runtime unsupported-rule mismatches were auto-classified as expected drift. The oracle classifier now leaves untriaged unsupported-rule mismatches as unexpected/release-blocking, and the checked-in T2 rows carry explicit expectedDriftUnsupportedRule notes. - ([f6b0a27](https://github.com/commit/f6b0a27490fbded1d477d592575423f6ce0c2046)) - Tyr Chen
- phase 22: package M9 G8 readiness hardening

Adds Makefile-discoverable standard and readiness gates, publishes the M9 G8 release-hardening snapshot, public readiness documentation, release notes, and key decision D17 covering specs 25, 27, and 28.

Exit criteria are met for evidence packaging and public scope documentation. The veraPDF-grade public claim remains intentionally blocked until the private T3 release-lab corpus gate satisfies specs/27-oracle-corpus-verification-plan.md. - ([e0256ad](https://github.com/commit/e0256adccbe8f44f3ac6f33527b90b3499ca57bb)) - Tyr Chen
- phase 22 review: enforce T3 release gate

Adds strict release-mode oracle summary checks for the T3 real-world corpus threshold, a Makefile readiness-release-gate target, and documentation for the public artifact refresh versus the actual release claim gate.

Verification: cargo test -p pdfv-core --example oracle_corpus_summary; strict clippy for oracle_corpus_summary; make readiness-gates; make standard-gates; make readiness-release-gate correctly fails on the public T2-only manifest with 0 T3 rows. - ([0b7244a](https://github.com/commit/0b7244aab66acbac8c545a674ec37fce0374d030)) - Tyr Chen
- phase 22 review: add T3 manifest generator

Adds a Makefile-discoverable oracle-t3-manifest target that scans a private corpus root, skips symlinks, validates canonical relative paths, bounds row count, and emits private t3RealWorld manifest rows for the strict release gate.

Verification: cargo test -p pdfv-core --example oracle_t3_manifest; strict clippy for oracle_t3_manifest; smoke-generated a T3 manifest from tests/fixtures; make standard-gates; release-mode oracle summary still fails on current T2-only evidence with 0 T3 rows. - ([94575d5](https://github.com/commit/94575d5628d3ba3533ce8cc0ca20c0bca4f10e3e)) - Tyr Chen

---
## [pdfv-v0.1.1](https://github.com/compare/pdfv-v0.1.0..pdfv-v0.1.1) - 2026-05-16

### Miscellaneous Chores

- bump version - ([b2eb3b0](https://github.com/commit/b2eb3b041c848fbfdef353775cfa3b4d58d9ff1a)) - Tyr Chen

### Other

- Update CHANGELOG.md - ([08a1e2a](https://github.com/commit/08a1e2a130d891ed9b80b7531361d9a856c0a849)) - Tyr Chen
- Prepare crates for publish - ([03e194b](https://github.com/commit/03e194ba88203204cb42910fa802f3f08bd3c6e1)) - Tyr Chen

---
## [pdfv-v0.1.0] - 2026-05-16

### Features

- add verapdf e2e tests - ([4beeb79](https://github.com/commit/4beeb7924ffae1067caa35b8c0bbc67d41105e6c)) - Tyr Chen

### Miscellaneous Chores

- init the project - ([c9714dc](https://github.com/commit/c9714dcbd5947330025504e0c76e43e56cdc7704)) - Tyr Chen
- add specs - ([77397c1](https://github.com/commit/77397c1ad5f51e016de36706b50c0e0d61cba9cc)) - Tyr Chen
- update docs - ([6526bd4](https://github.com/commit/6526bd4732193a16a33956c9913c16dd9cc673b0)) - Tyr Chen

### Other

- phase 0-1: land workspace contracts

Retires Phase 0 risks by documenting profile-expression IR coverage, stream resource-limit defaults, and the decision to remove the server placeholder in favor of apps/cli. Updates spec 61 and key decisions accordingly.

Implements Phase 1 public contracts in pdfv-core: toolchain pin, crate lint roots, typed validation options/resource limits, report and batch data models, parser facts, domain errors, JSON report writer, snapshot-style JSON tests, and the CLI workspace spine. Exit criteria are met by build, tests, nightly fmt, clippy, rustdoc, pedantic/security clippy, cargo-deny, and cargo-audit. - ([f204598](https://github.com/commit/f204598b8dc1968a5f402c43537d74ced450db6e)) - Tyr Chen
- phase 2-3: land parser and validation M0

Implements phase 2 parser M0 and phase 3 profile/validation engine M0 from specs 11, 12, 13, 70, and 72. Adds tolerant bounded PDF parsing for headers, COS objects, classic xref/trailer handling, streams and parse facts; adds built-in M0 profile rules, bounded rule IR evaluation, model wrappers, iterative traversal, unsupported-rule reporting, and Validator::validate_reader reports.

Verification passed: cargo build --workspace --all-targets; cargo test --workspace --all-targets; cargo +nightly fmt --all -- --check; cargo clippy --workspace --all-targets -- -D warnings; strict pedantic/boundary clippy; RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps; cargo deny check with existing unmatched-license-allowance warnings; cargo audit. - ([c13b0d6](https://github.com/commit/c13b0d68d3c14e09410835201f97a753980c2b91)) - Tyr Chen
- phase 4: land reporting and CLI M0

Implements Phase 4 from specs 20, 50, and 72: report format dispatch with compact JSON, pretty JSON, and text writers; batch summary construction; pdfv validate with format, flavour, profile, and max-failures handling; deterministic exit codes; and shared fixture-backed CLI/library tests.

Independent review found six valid in-phase issues. Fixed capped text counters, negative max-failures parsing, unsupported flavour rejection, M0 custom-profile usage handling, batch exit ordering, and shared fixture manifest coverage. cargo deny still reports only existing unmatched license allowance warnings. - ([6fb7aeb](https://github.com/commit/6fb7aebacc9efec16ef4a0a3081055f3e5dfbc0e)) - Tyr Chen
- phase 5: land M1 modern PDF coverage

Implements Phase 5 from specs/91-impl-plan.md: xref stream parsing, object stream expansion, bounded FlateDecode stream decoding, M1 model wrappers, fuzz targets, regression fixtures, and criterion performance measurements.

Review fixed P1 xref parsing and object-stream allocation bounds plus P2 stream byte sharing and active page traversal limits. Deferred the remaining lazy-wrapper architecture improvement to specs/93-improvements-review.md. - ([b5a7cfc](https://github.com/commit/b5a7cfcb5306fd360ff182996ada14b011adec2b)) - Tyr Chen
- phase 6: land profile breadth and batch CLI

Implements Phase 6 / M2-M3 scope from specs 12, 20, 50, 70, 72, 90, and 91. Adds bounded veraPDF XML profile import, custom profile loading behind the custom-profiles feature, explicit unsupported-rule reporting, additional model facts for converted rules, profile catalog listing, recursive batch validation, bounded jobs, YAML config, output files, redacted paths, and batch internal-error aggregation.

Adds JSON/config examples and a conformance fixture matrix. Independent review findings were fixed in-phase; cargo deny still reports only the existing unmatched-license-allowance warnings. - ([abaa304](https://github.com/commit/abaa3042383f048dc1ed60fd472df4e7bd4d02d6)) - Tyr Chen
- phase 5 deferred: resolve lazy model graph

Resolve the Phase 5 deferred validation graph finding by replacing precomputed ModelLinks slices with owned ModelObjectRef wrappers materialized from ModelGraph during traversal. Linked object expansion now receives remaining max_objects budget before enqueueing and child constructors cap materialization per link.

Adds validation-level regressions for M1 page/font/annotation/outputIntent/contentStream traversal, deferred rule replay, stable assertion contexts, and low max_objects rejection. Clears the deferred-findings backlog entry. - ([deab896](https://github.com/commit/deab896246235e6411e10419d17587e285fad7f6)) - Tyr Chen
- phase 7: land M4 XML compatibility report

Completes the M4 MRR/XML compatibility decision from spike-mrr-compatibility.md, updates reporting and CLI specs, and adds ReportFormat::Xml with streaming escaped XML output for single and batch validation reports. The CLI now exposes --format xml and keeps --format mrr as a deprecated compatibility alias.

Exit criteria met: XML/MRR decision recorded, XML output covered by core and CLI tests, README/report examples updated, and all required gates pass. Review found one in-phase XML character-validity issue; fixed by rejecting XML 1.0-forbidden text via ReportError::Xml. - ([891c647](https://github.com/commit/891c6475438a0e6f2d2b570be940f89a9d0baa90)) - Tyr Chen
- phase 8: land M4 advanced profile facts

Promote the default built-in profile to pdfv-m4, add executable page/font/annotation/output-intent/content-stream fact checks, and expose allowlisted direct dictionary properties for imported veraPDF-style expressions. Covers specs 12, 13, 20, 50, 72, 90, and 91 exit criteria for the remaining M4 profile-fact slice.

Review fixed one valid in-phase P2: dictionary property fallback was too broad and could turn derived veraPDF properties into null instead of unsupported. It is now allowlisted per model with regression coverage. Password/decryption remains unscoped future M4 work. - ([7eb4fcb](https://github.com/commit/7eb4fcb76389e51040f1b89d01f2641d7313ff9a)) - Tyr Chen
- phase 9: land M4 password decryption support

Implements the Phase 9 M4 password/decryption slice from specs 10, 11, 13, 14, 20, 50, 61, 70, 72, and 91. This adds redacted PasswordSecret plumbing, bounded CLI password sources, Standard security handler revision 2-4 parsing/authentication, RC4 and AESV2 object string/stream decryption, safe encrypted/parse-failed status handling, and encrypted fixture coverage.

Exit criteria covered by RC4 user-password, AESV2 owner-password, R3/R4 RC4, /EncryptMetadata false, missing/wrong password, unsupported revision/public-key handler, malformed /Encrypt, redaction, CLI password source tests, strict clippy, cargo audit, and cargo deny check. Independent review findings were fixed before commit. - ([f1f2ec9](https://github.com/commit/f1f2ec9fff0914efca56db96afbd187f30165469)) - Tyr Chen
- phase 10: land AES-256 decryption risk gate

Phase 10 publishes the AES-256 Standard security handler spike, updates the password/decryption design with implementation-ready R5/R6 dictionary invariants, Algorithm 2.A/2.B handling, AESV3 object decryption, /Perms validation, generated fixture requirements, and records the security/dependency decisions in specs 14, 61, 70, 72, and 99.

The gate is cleared for a dedicated future AES-256 implementation phase; no deferred finding was needed because revisions 5-6 are not recorded as permanently unsupported. - ([2593ef0](https://github.com/commit/2593ef081e0113fbebfbd342e6188d08c36ae478)) - Tyr Chen
- phase 10 review: tighten AESV3 details

Fix review findings by specifying PKCS#7-compatible padding for AESV3 string and stream content, aligning /Perms validation wording across revisions 5 and 6, and pointing qpdf documentation evidence at the current docs URL. - ([90f8fb2](https://github.com/commit/90f8fb2339a7f45a61a6696d30b5c9b944fcddd4)) - Tyr Chen
- phase 11: land AES-256 password decryption

Implements M4 AES-256 Standard security handler support for revisions 5 and 6, including validated R5/R6 encryption dictionaries, Algorithm 2.A/2.B owner and user authentication, AESV3 object decryption, and /Perms validation. Updates the roadmap and impl plan to add the dedicated Phase 11 unlocked by the AES-256 risk gate.

Adds sha2 as a scoped decryption dependency and deterministic generated fixtures covering R5/R6 user and owner passwords, wrong passwords, tampered /Perms, unsupported AES-256 crypt filters, malformed short key fields, and AESV3 string/stream decryption. Gates passed: build, test, fmt, clippy, strict clippy, docs, audit, deny, and diff check. - ([10c5183](https://github.com/commit/10c5183076e1cc7ebac48b99beb003bd90eda567)) - Tyr Chen
- phase 11 review: tighten AES-256 decryption

Fixes the independent review findings for Phase 11: uses a wider bounded R6 Algorithm 2.B round counter, applies decrypted stream limits after decryption rather than to AES ciphertext plus IV/padding, validates R5/R6 crypt filters as Identity or StdCF AESV3 with Length 32 and AuthEvent DocOpen, and adds direct CLI wrong-password coverage for AESV3 exit code 3.

Adds regressions for AESV3 ciphertext exceeding the decrypted-byte cap while plaintext fits, unsupported crypt-filter names, invalid AESV3 filter length/auth event, and CLI AESV3 wrong-password handling. Full quality gates passed after the fixes. - ([e6b4d43](https://github.com/commit/e6b4d43d6b94d43c2418b62fe86be48345e99caf)) - Tyr Chen
- phase 12: add parser filter and source parity

Implements the M5 parser parity slice across specs 11, 14, 15, 61, 70, 71, and 72. Adds the decoder registry, bounded ASCIIHex/ASCII85/RunLength/LZW/Flate predictor decoding, metadata-mode image filters, named Crypt composition with Standard-handler decryption, spill-file source storage over the memory threshold, and structured stream/xref facts for filter and /Prev/hybrid anomalies.

Verification covers decoder fixtures, named Crypt plus downstream Flate, spill-file parsing, xref chain facts, workspace gates, strict parser clippy, audit/deny, and parser benches. tempfile 3.27.0 was already checked as the current workspace dependency. - ([cb0f538](https://github.com/commit/cb0f538561f04ad410b03ab7c8225500ada47a1e)) - Tyr Chen
- add M5-M7 parity roadmap - ([fa23d3a](https://github.com/commit/fa23d3af610b3719b6bbdfe7bd2367d9626d29c6)) - Tyr Chen
- phase 13: land profile catalog parity

Generate a deterministic built-in catalog from the vendored veraPDF profiles, expose source pin and executable coverage in profile listing, expand PDF/A, PDF/UA, and WTPDF flavour selection, and extend the bounded rule IR for arithmetic, ternary, modulo, bounded regex, and collection built-ins. Unsupported official rules now carry citations as report data and force incomplete validation status.

Covers specs 10, 12, 16, 20, 50, 61, 70, and 72. Review findings fixed in-phase: exact PDF/UA-2 profile matching, citation propagation, deterministic unsupported accounting for nested paths, stable coverage columns, and per-profile load/list regression coverage. - ([626c383](https://github.com/commit/626c38306a300069b4c709ced3f1591967e9e507)) - Tyr Chen
- phase 14: broaden validation model registry

Land M6 validation model breadth across specs 15, 16, 17, 70, 71, and 72. Adds the internal model registry, generated-profile schema checks, broad generic validation families, bounded traversal coverage, official coverage regression checks, and a broad traversal benchmark.

Exit criteria covered: generated rules either bind to registered schema or remain unsupported with reasons; traversal remains iterative and capped; PDF/UA-2 and WTPDF executable coverage now reaches at least 90%; M6 family fixtures and parser benches pass. No deferred findings recorded before review. - ([5e21d69](https://github.com/commit/5e21d6989f8c96b5ef56861542f95812293874c4)) - Tyr Chen
- phase 14 review: fix model bounds and schema surface

Fix independent review findings for M6 validation model breadth: broad model roots now materialize only for families required by the selected profile, registry link schemas no longer advertise unimplemented traversal, the broad traversal benchmark fails hard on setup errors, and new registry/schema types stay internal to pdfv-core.

Re-ran build, tests, fmt, clippy, strict clippy, docs, audit, deny, profile generation determinism, and parser/traversal benches after the fixes. - ([33a1ce2](https://github.com/commit/33a1ce2886abee226d2e7b81b4de65ea52c697a2)) - Tyr Chen
- phase 15: add XMP flavour detection

Implement bounded catalog XMP extraction, namespace-aware identification parsing, and auto profile selection for PDF/A, PDF/UA, and WTPDF claims. Reports now carry structured XMP facts and auto-detection warnings without serializing metadata packets.

Covers Phase 15 tasks from specs 18, 17, 20, 70, and 72: absent or malformed XMP falls back with structured warnings, explicit selection remains supported, hostile DTD/entity input is rejected, and fixtures cover absent, malformed, single-claim, multi-claim, incompatible, and encrypted-metadata-adjacent validation paths. - ([9a3bc7f](https://github.com/commit/9a3bc7f4798b82de11401154be53e730a3082466)) - Tyr Chen
- phase 15 review: fix XMP facts and encrypted metadata

Parse bounded XMP evidence for explicit and encrypted validation paths, preserve malformed/hostile XMP facts, filter incompatible detected profile groups, scope namespace bindings, cap metadata decode bytes, and add encrypted metadata fixtures. - ([c0df0c9](https://github.com/commit/c0df0c91146b24fb5a223eae8074b131dc7c1573)) - Tyr Chen
- phase 16: add feature and policy reports

Implement M7 feature extraction and policy reporting across the core contracts, validation facade, JSON/XML/text writers, and CLI. Feature extraction is read-only over the Phase 14 model families, policy evaluation consumes FeatureReport data only, and validate exposes --extract plus --policy-file.

Exit criteria are covered by CLI integration tests for JSON feature reports, XML policy merging, failing policy status, and unknown feature-family rejection. Added docs/research/spike-policy-language.md and indexed it; no deferred findings yet. - ([9eaf69f](https://github.com/commit/9eaf69f8aaa0e43bc75c1c3ceb8be527cf5826e7)) - Tyr Chen
- phase 16 review: harden feature and policy reports

Fix all five independent review findings: redact content-bearing feature strings, bound policy-file reads, validate policy schema before worker execution, emit typed XML feature values, and truncate feature extraction cleanly on object-cap exhaustion.

Added regression coverage for content redaction, feature truncation, invalid policy schemas, and oversized policy files. Standard gates, strict clippy, rustdoc, cargo audit, and cargo deny check pass; deny still reports the existing warnings only. - ([fee5946](https://github.com/commit/fee5946f92e9c7388f674d138c19bd1fe6de9069)) - Tyr Chen
- phase 17: add repair and report parity surfaces

Adds RepairReport contracts, explicit repair refusal reports, the non-in-place repair-metadata command with atomic output writes, and raw XML/static HTML report formats for validation and repair outputs. Covers specs 10, 19, 20, 50, 70, and 72, including safe output directory validation and no literal password compatibility documentation.

Known follow-ups remain in the planned metadata rewrite scope: current repair support copies valid single-profile inputs unchanged and refuses unsupported repair states rather than rewriting metadata objects. - ([e7c4fbf](https://github.com/commit/e7c4fbfe3a7409279b4652a80f660192ba170309)) - Tyr Chen
- phase 17 review: harden repair and raw reports

Moves repair execution into pdfv-core, fixes raw XML task metadata for feature/policy/repair outputs, adds full raw/HTML golden assertions, preserves existing outputs on repair refusal, aligns config format spelling with CLI raw/html, and removes the warning-message fallback.

Review findings fixed: P1 repair core API boundary, P1 raw task metadata, P1 golden coverage, P2 failed/pre-existing output coverage, P2 config format mismatch, and P3 warning fallback. - ([bb4faf1](https://github.com/commit/bb4faf1a48f8156aa961cd61ae2e9bac6a0d7f12)) - Tyr Chen
- phase 18: close verapdf cli parity gaps

Adds the missing default flavour fallback and non-PDF recursive discovery surfaces from the veraPDF CLI/product parity specs, including migration aliases for --defaultflavour, --recurse, and --nonpdfext. Updates compatibility docs and stale report/CLI spec text to match the Phase 17 raw/HTML report surface.

Review found one in-phase issue: default flavour was silently ignored with custom profiles. The CLI now rejects that ambiguous combination and covers it with an integration test. No deferred findings. - ([4724da8](https://github.com/commit/4724da80c2e66c8e34fba03b1ca6aa49d53b67a1)) - Tyr Chen
- Fix rustfmt CI failure - ([30d5529](https://github.com/commit/30d55292802ce0495133510fb8d13f54bffa8961)) - Tyr Chen

<!-- generated by git-cliff -->
