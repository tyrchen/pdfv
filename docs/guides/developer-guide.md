# pdfv Developer Guide

[简体中文](developer-guide.zh-CN.md)

Status: current for Phase 18.

This guide covers local development, repository layout, generation tasks, quality gates, and contribution expectations.

## Repository Layout

```text
apps/cli/        CLI adapter over pdfv-core
crates/core/     parser, model, validation, repair, report, and public library APIs
docs/            user-facing docs and research notes
docs/guides/     user and developer guides
specs/           product, design, security, testing, and implementation specs
tests/fixtures/  shared PDF fixtures
vendors/         vendored veraPDF references
fuzz/            fuzzing workspace
```

The CLI should remain a thin adapter. Parser, validation, report, feature, policy, and repair behavior belongs in `pdfv-core`.

## Toolchain

The repository pins the Rust toolchain in `rust-toolchain.toml`. Use the pinned stable toolchain for build/test and nightly for formatting:

```bash
rustup toolchain install stable
rustup toolchain install nightly
cargo --version
cargo +nightly fmt --version
```

Install required cargo tools:

```bash
cargo install cargo-audit
cargo install cargo-deny
```

## Build and Test

Run the full workspace build:

```bash
cargo build --workspace --all-targets
```

Run all tests and benchmark harness compile checks:

```bash
cargo test --workspace --all-targets
```

Run a focused CLI integration test:

```bash
cargo test -p pdfv --test validate test_should_validate_pdf_and_emit_text_report
```

Run a focused core test:

```bash
cargo test -p pdfv-core parser::tests::test_should_parse_header_and_catalog_from_m0_fixture
```

## Required Gates

Before handing off a change, run:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

`cargo deny check` currently emits known warnings for unmatched allowed licenses and duplicate `wit-bindgen` lock entries; it must still exit successfully.

## Makefile Targets

Use existing targets when they match the task:

```bash
make build
make test
make generate-profiles
make check-agent-sync
```

Add new automation as Makefile targets instead of ad hoc scripts.

## Profile Generation

Generated built-in profile data lives in `crates/core/src/generated_profiles.rs`.

Regenerate after updating vendored profile inputs:

```bash
make generate-profiles
```

Then run the profile tests:

```bash
cargo test -p pdfv-core profile::tests::test_should_load_and_validate_every_generated_builtin_profile
cargo test -p pdfv --test validate test_should_validate_with_every_phase_13_builtin_profile
```

## Documentation

User-facing docs live under `docs/`; guides live under `docs/guides/`. When adding or renaming documentation:

- update `docs/index.md`;
- add a zh-CN counterpart for README and guide-level docs;
- keep examples aligned with `pdfv validate --help` and integration tests;
- prefer links to specs/research for design rationale instead of duplicating long design text.

## Coding Standards

Project rules are defined in `AGENTS.md`. High-impact rules:

- no `unsafe`;
- no `unwrap()` or `expect()` in production code;
- no `todo!()`, `unimplemented!()`, or incomplete placeholders;
- public items require docs;
- use structured error types in the library and `anyhow` context in the CLI;
- validate hostile input at the boundary and enforce explicit resource limits;
- keep generated/profile/report behavior deterministic.

## CLI Work

When changing CLI behavior:

- keep parser/validation logic in `pdfv-core`;
- add or update `apps/cli/tests/validate.rs`;
- document stable exit code behavior;
- do not add literal password arguments;
- reject unsupported or ambiguous option combinations at the clap/config boundary.

## Report Work

Report writers are library APIs. Keep these properties:

- JSON uses camelCase and stable serde shapes;
- XML/Raw/HTML writers escape text and attributes;
- reports never include passwords, keys, decrypted bytes, or raw PDF content;
- golden-style tests should cover new output sections.

## Dependency Work

Before adding a dependency:

- prefer workspace dependencies;
- choose maintained pure-Rust crates where possible;
- pin versions consistently with existing dependency policy;
- run `cargo audit` and `cargo deny check`;
- update docs/specs if the dependency changes user-facing behavior or security posture.

## Commit Shape

Keep commits focused. For planned phase work, commit messages should name the phase and summarize specs/exit criteria. For narrow documentation or bug-fix work, use a direct summary and list the verification commands run.
