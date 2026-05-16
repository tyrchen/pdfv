# pdfv

`pdfv` is a Rust PDF conformance validator with a library-first core and a CLI for local files, repositories, and CI jobs.

The current product validates PDF/A-oriented profiles against a bounded parser and rule engine, emits deterministic reports, and keeps hostile input behind explicit resource limits.

## Install From Source

```bash
cargo install --path apps/cli
```

## CLI

Validate one file and emit JSON:

```bash
pdfv validate tests/fixtures/minimal-valid.pdf --format json
```

Emit human-readable text:

```bash
pdfv validate tests/fixtures/minimal-valid.pdf --format text
```

Emit XML compatibility output for veraPDF-style machine-readable workflows:

```bash
pdfv validate tests/fixtures/minimal-valid.pdf --format xml
```

Validate a directory recursively with bounded parallelism and redact paths:

```bash
pdfv validate ./documents --recursive --jobs 4 --redact-paths --format json --output report.json
```

List built-in profiles:

```bash
pdfv profiles list
```

Exit codes are stable for automation:

| Code | Meaning |
| --- | --- |
| 0 | all processed files are valid |
| 1 | validation completed and at least one file is invalid |
| 2 | at least one file could not be parsed |
| 3 | at least one file is encrypted and unsupported |
| 4 | validation is incomplete because required rules are unsupported |
| 64 | invalid CLI arguments or config |
| 70 | internal processing failure |

## Config

Runtime defaults can live in YAML:

```yaml
validation:
  flavour: auto
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

The standard gates are:

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

## License

This project is distributed under the terms of the Mozilla Public License 2.0.

See [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for vendored veraPDF license notices.

See [LICENSE](LICENSE.md) for details.
