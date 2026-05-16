# 80-glossary: Terms

Status: draft · Owner: pdfv · Last updated: 2026-05-15

## COS object

A low-level PDF object: null, boolean, number, name, string, array, dictionary, stream, or indirect reference. `CosObject` is parser-owned.

## PD object

A higher-level PDF domain wrapper such as document, page, catalog, metadata, font, annotation, action, or output intent. Validation model objects may wrap COS or PD data.

## Parse fact

A structured fact discovered during tolerant parsing, such as header offset or stream EOL compliance. Parse facts are rule inputs and report data, not logs.

## Validation profile

A rule set for a specific conformance flavour such as PDF/A-1b. Profiles contain rules, variables, metadata, and references.

## Rule IR

The bounded Rust expression representation compiled from profile rule tests. It replaces in-process JavaScript execution.

## Validation session

Per-input mutable state: parsed document, caches, traversal stack, variables, counters, and evaluator state. It replaces global/thread-local state.

## Assertion

One executed rule result against one validation model object. Failed assertions may be capped in detail, but counters stay complete.

## Incomplete validation

A run that parsed and executed available rules but could not evaluate required rules because they are unsupported. It is not compliant.

## MRR

veraPDF’s machine-readable XML report. It is a compatibility target for a later milestone, not M0.

## Standard security handler

The password-based PDF encryption handler identified by `/Filter /Standard` in the encryption dictionary. pdfv's first password phase supports revisions 2-4 only.

## Password secret

A redacted, non-serializable wrapper around a caller-supplied PDF password. It may be exposed only inside the decryption module and must never appear in reports, logs, config files, or `Debug` output.

## Crypt filter

A PDF encryption dictionary entry that selects how strings or streams are encrypted. Phase 9 supports the Standard handler's direct crypt filters for RC4 and AESV2 plus identity handling for `/Crypt` streams.
