# 72-testing-strategy: Verification Plan

Status: draft · Owner: pdfv · Depends on: all component specs

## 1. Test pyramid

- Unit tests: byte parsers, object accessors, rule IR parsing/evaluation, report summaries.
- Integration tests: whole-file validation fixtures and CLI exit/output tests.
- Property tests: token parser invariants, object key parsing, bounded expression evaluation.
- Fuzz tests: parser entrypoint and stream parser.
- Snapshot tests: stable JSON reports and human text output.
- Conformance fixtures: curated PDFs from public suites or generated local fixtures, with license metadata tracked.
- Encrypted fixtures: generated or licensed PDFs covering supported and unsupported password/decryption paths.
- Parity metrics: generated coverage reports for imported rules, executable rules, bound rules, unsupported reasons, model families, feature families, and corpus agreement as defined in [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md).

## 2. Fixture policy

Fixtures live under `tests/fixtures/` with a manifest naming source, license, expected parse status, expected validation status, and reason. Generated fixtures include source generator code. Large fixtures are gated to ignored tests if they slow normal `cargo test`.

Password/decryption fixtures must include: no password, wrong password, correct user password, correct owner password, RC4 revision 2/3/4 where available, AESV2 revision 4, unencrypted metadata handling, unsupported revision 5/6, unsupported public-key handler, malformed `/Encrypt`, encrypted string objects, and encrypted streams. Future AES-256 coverage must add generated R5/R6 AESV3 user-password and owner-password fixtures, tampered `/Perms`, and malformed short `/O`, `/U`, `/OE`, `/UE`, and `/Perms` fields. Fixture manifests must not contain real user passwords from external documents; generated passwords are test-only.

## 3. Required test names

Follow AGENTS.md Testing: descriptive `test_should_*` names. Use `rstest` for matrices and `proptest` for parser invariants.

## 4. Golden outputs

JSON golden outputs are canonical and semver-relevant. Text golden outputs may change within a minor version unless documented as stable. Golden updates require a review note explaining the behavioural change.

## 5. Fuzzing

M1 introduces `cargo fuzz` targets:

- `fuzz_parse_document`
- `fuzz_parse_stream_object`
- `fuzz_parse_encryption_dictionary`
- `fuzz_rule_expr`
- `fuzz_parse_content_stream`
- `fuzz_structure_tree`

Fuzz failures become regression fixtures before fixes are accepted.

## 6. Parity gates

The parity phases add Makefile-discoverable checks from [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md):

- `make parity-profile-report` records imported, lowered, bound, and unsupported rules by flavour.
- `make parity-model-schema` verifies generated profile object/property/link references against the model registry.
- `make parity-corpus` runs generated and checked-in semantic agreement rows; live veraPDF oracle rows remain ignored unless explicitly enabled.

Coverage decreases require a review note. Placeholder zero-count parity reports are forbidden at milestone exits.

## 7. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md), [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md), [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md)
- → Constrains: [91-impl-plan.md](./91-impl-plan.md)
