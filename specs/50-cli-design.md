# 50-cli-design: Command Line Interface

Status: draft · Owner: pdfv · Depends on: [20-reporting-design.md](./20-reporting-design.md)

## 1. Purpose

The CLI is a thin adapter over `pdfv-core`. It handles argument parsing, YAML config, bounded file discovery, parallel execution, report output, and exit codes. It does not contain parser or validation logic.

## 2. Commands

```text
pdfv validate <paths>...
    --flavour <auto|pdfa-1b|pdfa-2b|...>
    --default-flavour <flavour>
    --profile <path>
    --format <json|json-pretty|text|xml|mrr>
    --output <path>
    --recursive
    --jobs <N>
    --max-failures <N|-1>
    --record-passes
    --config <path>
    --redact-paths
    --password-stdin
    --password-file <path>
    --password-env <ENV_VAR>

pdfv profiles list
pdfv --version
```

`clap 4.6.1` is the current CLI parser candidate, verified with `cargo search` on 2026-05-15.

## 3. Exit codes

| Code | Name | Meaning |
| --- | --- | --- |
| 0 | valid | All processed PDFs validated and conformed. |
| 1 | invalid | All processed PDFs were processed, at least one failed validation. |
| 2 | parse-failed | At least one file could not be parsed as PDF. |
| 3 | encrypted | At least one encrypted file could not be validated. |
| 4 | incomplete | At least one required rule was unsupported. |
| 64 | usage | Invalid CLI arguments or config. |
| 70 | internal | Unexpected internal failure. |

Worst category wins for batch runs. This mirrors veraPDF’s explicit exit-code style while adding an incomplete category for unsupported Rust rule coverage.

## 4. Config

Runtime-tunable defaults live in YAML via the `config 0.15.23` crate. CLI flags override config values. Config deserialization uses `serde(deny_unknown_fields)` and immediate validation.

```yaml
validation:
  flavour: auto
  maxFailedAssertionsPerRule: 100
  recordPassedAssertions: false
resources:
  maxFileBytes: 1073741824
  maxStreamDecodeBytes: 67108864
output:
  format: json
```

`output.format: xml` selects the XML compatibility report. `mrr` is accepted on the CLI as a deprecated alias for `xml`; configs should use `xml` so machine-readable output names align with current veraPDF documentation.

Password config follows [14-password-decryption-design.md](./14-password-decryption-design.md): config may identify a password source (`stdin`, file path, or environment variable name) but must never contain a literal password value.

## 5. Password input

The CLI deliberately does not provide `--password <text>` because literal arguments leak through shell history, process listings, CI logs, and crash diagnostics.

Exactly one of `--password-stdin`, `--password-file`, or `--password-env` may be used. The resolved password is passed to `pdfv-core` as a redacted `PasswordSecret`; it is never printed, traced, serialized into JSON/XML/text reports, or included in `Debug`.

## 6. Concurrency

The CLI runs one `ValidationSession` per file with bounded parallelism. The first implementation may use `rayon 1.12.0` for synchronous file parallelism or Tokio `spawn_blocking` if a future async service shares the binary. Shared state is limited to immutable profiles and report aggregation through message passing.

## 7. AGENTS.md binding

- Error Handling: CLI uses `anyhow` with context and maps failures to exit codes.
- Async & Concurrency: bounded workers only; task panics are caught by join handling and reported as internal errors.
- Safety & Security: path traversal rules matter for config/output paths; no shell execution with user input.
- Cryptography & Secrets: password sources are redacted and never logged; CLI rejects literal password arguments.
- Serialization: YAML config validates at load; JSON output uses core report types.
- Testing: `assert_cmd`-style integration tests for exit codes and output; no over-mocking.
- Logging & Observability: `tracing-subscriber` human-readable default, JSON logs when configured; no `println!` outside final user output.
- Performance: directory walks are bounded and skip non-PDF files unless explicitly configured.
- Documentation: `--help` examples align with PRD quickstart.

## 8. Cross-references

- ← Depends on: [20-reporting-design.md](./20-reporting-design.md)
- → Consumed by: [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- ↔ Related design: [14-password-decryption-design.md](./14-password-decryption-design.md)
- ↔ Related research: veraPDF CLI is a config adapter over processor config and chooses single/multi-process execution (`vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/VeraPdfCli.java:111`, `vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/commands/VeraCliArgParser.java:572`).
- ↔ Related research: [../docs/research/spike-mrr-compatibility.md](../docs/research/spike-mrr-compatibility.md)
