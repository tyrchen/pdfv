# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

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
