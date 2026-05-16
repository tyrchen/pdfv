# Study: veraPDF E2E Tests as pdfv Oracles

Status: Done · Owner: pdfv · Date: 2026-05-16 · Vendor pins:
`vendors/veraPDF-apps` @ `c6f3531d81b203e0426e63d60a4080b91b219dd0`,
`vendors/veraPDF-validation` @ `48b88fbeb8731c5489d9fc3f916bf91a22153508`,
`vendors/veraPDF-parser` @ `5164caf280a1c24b3d6335f87ee4319ca0443873`,
`vendors/veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`

## Why this study

We need to know whether the vendored veraPDF test code can raise `pdfv` confidence without copying a Java test suite wholesale. The specific question is: which upstream end-to-end tests or fixtures are useful as `pdfv` CLI/library oracles, and how should they be wired into Rust tests?

## Architecture map

```text
veraPDF-apps
  tests/exit-status.sh
    downloads veraPDF-corpus
    runs the installed CLI on selected pass/fail PDFs
    asserts process exit codes
  cli/src/test/java
    JUnit tests for CLI config mapping, task selection, threading, temp files
    small checked-in PDF fixtures

veraPDF-validation
  feature-reporting/src/test/java
    extracts feature reports from object-focused PDFs
    compares feature tree nodes against text goldens
  feature-reporting/src/test/resources/objects
    PDFs and result text files for annotation/font/page/color-space/etc.

veraPDF-parser
  src/test/java and src/test/resources
    parser unit fixtures, including xref and filter coverage
```

`pdfv` already has Rust CLI integration tests for valid, invalid, parse-failed, batch, XML/MRR/raw/html, feature extraction, config, metadata repair, and encrypted cases in `apps/cli/tests/validate.rs:43`. Existing checked-in fixture provenance is tracked in `tests/fixtures/manifest.md:3`, and the conformance matrix explicitly says normal tests are currently generated in-repo rather than dependent on external corpora (`docs/conformance-fixture-matrix.md:5`).

## Hot path walkthrough

The only true upstream E2E shell test is `vendors/veraPDF-apps/tests/exit-status.sh`. It downloads `veraPDF-corpus` from GitHub if absent (`vendors/veraPDF-apps/tests/exit-status.sh:32`), chooses the CLI from `$VERAPDF` or `./verapdf/verapdf` (`vendors/veraPDF-apps/tests/exit-status.sh:38`), then validates selected PDF/A-1b corpus files for single pass, single fail, batch pass, batch fail, and mixed-directory behavior (`vendors/veraPDF-apps/tests/exit-status.sh:42`, `vendors/veraPDF-apps/tests/exit-status.sh:44`, `vendors/veraPDF-apps/tests/exit-status.sh:46`, `vendors/veraPDF-apps/tests/exit-status.sh:48`, `vendors/veraPDF-apps/tests/exit-status.sh:50`). It also checks bad parameters, an artificial memory-pressure case, and parse failure (`vendors/veraPDF-apps/tests/exit-status.sh:52`, `vendors/veraPDF-apps/tests/exit-status.sh:54`, `vendors/veraPDF-apps/tests/exit-status.sh:58`). Expected exit codes are hard-coded as valid `0`, validation failure `1`, bad params `2`, out-of-memory `3` or `1`, and parse error `7` (`vendors/veraPDF-apps/tests/exit-status.sh:62`).

This is directly useful as a behavior checklist, but not as a drop-in test. `pdfv` intentionally uses different process codes today: valid `0`, invalid `1`, parse failed `2`, encrypted `3`, incomplete `4`, usage `64`, and internal `70` (`apps/cli/src/main.rs:31`). Therefore the shell script should be translated into a `pdfv`-native matrix, not executed unchanged.

The Java CLI tests are mostly config-adapter tests, not file-level E2E. `VeraPdfCliProcessorTest` checks default XML output and every `FormatOption` mapping (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/VeraPdfCliProcessorTest.java:64`), `--passed`/`--success` mapping to pass recording (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/VeraPdfCliProcessorTest.java:93`), feature extraction flags (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/VeraPdfCliProcessorTest.java:122`), multiple feature names (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/VeraPdfCliProcessorTest.java:142`), and config-vs-CLI precedence (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/VeraPdfCliProcessorTest.java:166`). `VeraCliTasksTest` checks validate, extract, repair, and combinations of those tasks (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/commands/VeraCliTasksTest.java:51`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/commands/VeraCliTasksTest.java:63`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/commands/VeraCliTasksTest.java:74`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/commands/VeraCliTasksTest.java:85`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/cli/commands/VeraCliTasksTest.java:97`). These are useful as a coverage checklist for Rust CLI tests, but many equivalent cases already exist in `apps/cli/tests/validate.rs`.

There are two local upstream fixture families worth reusing later. First, `MultithreadingTest` runs four parallel validators over `veraPDFtest-pass-a.pdf` and compares compliance, assertions, and total assertion counts across threads (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/apps/test/MultithreadingTest.java:39`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/apps/test/MultithreadingTest.java:50`, `vendors/veraPDF-apps/cli/src/test/java/org/verapdf/apps/test/MultithreadingTest.java:60`). `pdfv` has a `--jobs` flag (`apps/cli/src/main.rs:115`), so this maps well to a deterministic parallel-batch consistency test once `pdfv` can validate richer corpus files.

Second, `FeatureTester` maps ten object-specific PDFs to feature families such as annotations, color spaces, fonts, forms, info dictionary, outlines, pages, and ICC profiles (`vendors/veraPDF-validation/feature-reporting/src/test/java/org/verapdf/features/gf/FeatureTester.java:41`). It extracts only the selected family and compares normalized feature-tree strings against text goldens (`vendors/veraPDF-validation/feature-reporting/src/test/java/org/verapdf/features/gf/FeatureTester.java:64`, `vendors/veraPDF-validation/feature-reporting/src/test/java/org/verapdf/features/gf/FeatureTester.java:71`, `vendors/veraPDF-validation/feature-reporting/src/test/java/org/verapdf/features/gf/FeatureTester.java:105`). This is more valuable than the CLI JUnit tests for future `pdfv` feature extraction parity because the fixtures are local and organized by object family.

## Key data structures

Exit-code matrix: `exit-status.sh` is the upstream process-contract source. It should be represented in `pdfv` as data rows `{case_name, inputs, args, expected_pdfv_exit, expected_status_counts}` rather than as a shell port. The rows should keep the upstream corpus path and upstream expected code for traceability.

Fixture manifest: `pdfv` requires checked-in fixtures to name source, license, expected parse status, expected validation status, and reason (`tests/fixtures/manifest.md:3`). Any vendored PDF copied into `tests/fixtures` must get a manifest row. For external `veraPDF-corpus` PDFs, prefer a separate ignored/network test that downloads to a cache and records corpus commit or archive digest instead of silently adding binaries.

Feature family golden: `FeatureTester` normalizes away unstable `id` attributes before comparison (`vendors/veraPDF-validation/feature-reporting/src/test/java/org/verapdf/features/gf/FeatureTester.java:89`). If `pdfv` compares feature output against veraPDF, it should use semantic JSON assertions over selected fields, not byte-identical XML/text output, because generated IDs and ordering are likely implementation-specific.

Parallel consistency result: upstream compares compliance, assertion set, and total assertion count across threads (`vendors/veraPDF-apps/cli/src/test/java/org/verapdf/apps/test/MultithreadingTest.java:60`). `pdfv` should compare `ValidationStatus`, summary counts, warning counts, and stable assertion IDs across `--jobs 1` and `--jobs N`.

## What we will adopt

Adopt the upstream E2E cases as a `pdfv` CLI conformance checklist: single valid file, single invalid file, all-valid batch, all-invalid batch, mixed batch/directory, usage error, resource-limit failure, and parse failure. Do not copy the exact exit codes, because `pdfv` uses BSD-style usage/internal codes and a distinct parse-failed code.

Adopt an ignored, opt-in corpus test target rather than putting network work in normal `cargo test`. The test should download or use a preexisting `veraPDF-corpus` checkout only when explicitly enabled, then run a small stable slice equivalent to `exit-status.sh`. Normal test fixtures should remain local and provenance-tracked as the current conformance matrix requires.

Adopt the feature-reporting object fixtures as future parity fixtures when `pdfv` feature extraction matures. Start with `InfoDictionary.pdf` and `Pages.pdf`, because `pdfv` already has feature-selection CLI coverage (`apps/cli/tests/validate.rs:216`) and the upstream fixture family gives object-specific expected behavior.

Adopt a parallel consistency test modelled on `MultithreadingTest`: validate the same fixture set with `--jobs 1` and `--jobs 4`, then compare stable summaries and assertion IDs. This targets `pdfv`'s actual concurrency surface instead of veraPDF's Java executor internals.

Add Makefile targets if this becomes executable automation. The likely split is `make test` for normal local tests and a new explicit target such as `make test-conformance-verapdf` for ignored/network corpus tests, consistent with the project rule that automation belongs in the Makefile.

## What we will avoid

Avoid executing `vendors/veraPDF-apps/tests/exit-status.sh` directly against `pdfv`. It assumes a different command shape, different exit-code contract, `/tmp` working directory mutation, network download during test execution, and a Java memory-pressure behavior that does not map cleanly to Rust.

Avoid copying large or externally sourced corpus PDFs into `tests/fixtures` without a fixture-manifest update and license review. The vendored veraPDF code is dual GPL/MPL in headers, and fixture redistribution needs to be tracked explicitly rather than inferred from source-code licensing.

Avoid asserting byte-identical veraPDF XML/MRR output for broad E2E parity. Use semantic checks first: file status, profile/flavour, assertion IDs, failed/passed counts, parse status, and selected feature facts. Byte-level output compatibility belongs to a separate MRR/XML compatibility gate.

Avoid porting Java config-unit tests one-for-one. `pdfv` already has Rust integration coverage for several equivalent user-visible paths; missing cases should be added as idiomatic Rust tests around `assert_cmd`, not as a Java-test taxonomy.

## Recommended next implementation

1. Add `tests/verapdf_corpus.rs` as ignored integration tests gated by an environment variable such as `PDFV_VERAPDF_CORPUS_DIR`.
2. Add `make test-conformance-verapdf` to run `cargo test --test verapdf_corpus -- --ignored` with a clear error if the corpus directory is absent.
3. Encode the `exit-status.sh` cases as Rust rows, translating expected outcomes to `pdfv`'s current exit contract.
4. Add local fixture-manifest rows only for any PDFs copied from `vendors/`; leave external corpus files in their own cache/checkout.
5. Later, add feature-extraction parity rows for `vendors/veraPDF-validation/feature-reporting/src/test/resources/objects/pdf/*.pdf` once `pdfv` exposes the corresponding object facts.

## Open questions

`spike-verapdf-corpus-license.md`: What is the exact redistribution license and stable pinning strategy for `veraPDF-corpus` PDFs if we want to check any of them into `pdfv`?

`spike-verapdf-oracle-runner.md`: Should CI run `pdfv` only against known expected statuses, or also run upstream veraPDF as a live oracle and compare semantic reports?
