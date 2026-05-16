# pdfv

English | [简体中文](README.zh-CN.md)

`pdfv` is a Rust PDF conformance validator with a library-first core and a CLI for local files, repositories, and CI jobs.

The current product validates PDF/A-oriented and imported veraPDF profiles with a bounded parser and rule engine, emits deterministic reports, extracts bounded feature/policy data, and keeps hostile input behind explicit resource limits.

## Quick Start

Install from the workspace:

```bash
cargo install --path apps/cli
```

Validate one file:

```bash
pdfv validate tests/fixtures/minimal-valid.pdf --format text
```

Write a machine-readable report:

```bash
pdfv validate ./documents --recursive --jobs 4 --format json --output report.json
```

List built-in profiles:

```bash
pdfv profiles list
```

Repair metadata to a separate output directory:

```bash
pdfv repair-metadata input.pdf --output-dir repaired --prefix fixed- --format json
```

## Documentation

- [User Guide](docs/guides/user-guide.md)
- [Developer Guide](docs/guides/developer-guide.md)
- [JSON and Config Examples](docs/json-examples.md)
- [veraPDF CLI Compatibility](docs/verapdf-cli-compatibility.md)
- [Documentation Index](docs/index.md)

Chinese documentation:

- [用户指南](docs/guides/user-guide.zh-CN.md)
- [开发者指南](docs/guides/developer-guide.zh-CN.md)

## CLI Surface

`pdfv validate` supports:

- Report formats: `json`, `json-pretty`, `text`, `xml`, `mrr`, `raw`, `html`.
- Profile selection: `--flavour`, `--default-flavour`, `--profile`.
- Batch discovery: `--recursive`, `--non-pdf-extension`, `--jobs`.
- Reporting controls: `--output`, `--redact-paths`, `--record-passes`, `--max-failures`.
- Password sources: `--password-stdin`, `--password-file`, `--password-env`.
- Feature and policy reports: `--extract`, `--policy-file`.

Exit codes are stable for automation:

| Code | Meaning |
| --- | --- |
| 0 | all processed files are valid |
| 1 | validation completed and at least one file is invalid |
| 2 | at least one file could not be parsed |
| 3 | at least one file is encrypted and could not be validated |
| 4 | validation is incomplete because required rules are unsupported |
| 64 | invalid CLI arguments or config |
| 70 | internal processing failure |

## Config

Runtime defaults can live in YAML:

```yaml
validation:
  flavour: auto
  defaultFlavour: pdfa-1b
  recordPassedAssertions: false
resources:
  maxFileBytes: 268435456
  maxObjects: 1000000
  maxObjectDepth: 128
  maxArrayLen: 65536
  maxDictEntries: 16384
  maxNameBytes: 127
  maxStringBytes: 1048576
  maxStreamDeclaredBytes: 134217728
  maxStreamDecodeBytes: 268435456
  maxParseFacts: 100000
output:
  format: json
  path: report.json
  redactPaths: true
```

Use it with:

```bash
pdfv validate --config pdfv.yaml ./documents --recursive
```

## Library

```rust
use pdfv_core::{ReportFormat, Validator};

let validator = Validator::default();
let report = validator.validate_path("tests/fixtures/minimal-valid.pdf")?;
ReportFormat::JsonPretty.write_report(&report, std::io::stdout())?;
# Ok::<(), pdfv_core::PdfvError>(())
```

## Development

Required local gates:

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

See the [Developer Guide](docs/guides/developer-guide.md) for workflow details.

## License

This project is distributed under the terms of the Mozilla Public License 2.0.

See [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for vendored veraPDF license notices.

See [LICENSE](LICENSE.md) for details.
