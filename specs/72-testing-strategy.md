# 72-testing-strategy: Verification Plan

Status: draft · Owner: pdfv · Depends on: all component specs

## 1. Test pyramid

- Unit tests: byte parsers, object accessors, rule IR parsing/evaluation, report summaries.
- Integration tests: whole-file validation fixtures and CLI exit/output tests.
- Property tests: token parser invariants, object key parsing, bounded expression evaluation.
- Fuzz tests: parser entrypoint and stream parser.
- Snapshot tests: stable JSON reports and human text output.
- Conformance fixtures: curated PDFs from public suites or generated local fixtures, with license metadata tracked.

## 2. Fixture policy

Fixtures live under `tests/fixtures/` with a manifest naming source, license, expected parse status, expected validation status, and reason. Generated fixtures include source generator code. Large fixtures are gated to ignored tests if they slow normal `cargo test`.

## 3. Required test names

Follow AGENTS.md Testing: descriptive `test_should_*` names. Use `rstest` for matrices and `proptest` for parser invariants.

## 4. Golden outputs

JSON golden outputs are canonical and semver-relevant. Text golden outputs may change within a minor version unless documented as stable. Golden updates require a review note explaining the behavioural change.

## 5. Fuzzing

M1 introduces `cargo fuzz` targets:

- `fuzz_parse_document`
- `fuzz_parse_stream_object`
- `fuzz_rule_expr`

Fuzz failures become regression fixtures before fixes are accepted.

## 6. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md), [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md)
- → Constrains: [91-impl-plan.md](./91-impl-plan.md)

