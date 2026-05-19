# pdfv User Guide

[简体中文](user-guide.zh-CN.md)

Status: current for Phase 18.

This guide covers day-to-day CLI usage for validation, reports, feature extraction, policy checks, password-protected PDFs, and conservative metadata repair.

## Install

From this repository:

```bash
cargo install --path apps/cli
```

Confirm the binary is available:

```bash
pdfv --version
```

## Validate Files

Validate one file and print a human-readable summary:

```bash
pdfv validate document.pdf --format text
```

Validate multiple files and write compact JSON:

```bash
pdfv validate a.pdf b.pdf --format json --output report.json
```

Validate a directory recursively with bounded parallelism:

```bash
pdfv validate ./documents --recursive --jobs 4 --format json --output report.json
```

By default recursive discovery includes only files with a `.pdf` extension. To include files without that extension:

```bash
pdfv validate ./incoming --recursive --non-pdf-extension --format json
```

## Select Profiles

Let pdfv detect the flavour from XMP metadata and fall back to PDF/A-1b:

```bash
pdfv validate document.pdf --flavour auto --default-flavour pdfa-1b
```

Select a built-in flavour explicitly:

```bash
pdfv validate document.pdf --flavour pdfa-2b --format json
```

Use a custom profile XML:

```bash
pdfv validate document.pdf --profile profile.xml --format json
```

List built-in profiles and coverage metadata:

```bash
pdfv profiles list
```

## veraPDF Readiness

pdfv currently imports, lowers, and binds the vendored PDF/A, PDF/UA, and WTPDF
profile rules recorded in the M9 G8 readiness snapshot. The first public
veraPDF-grade claim is still withheld until the private T3 real-world corpus
gate passes.

See [veraPDF Readiness](../verapdf-readiness.md) for the supported profile
scope, corpus evidence, known drift, and out-of-scope surfaces.

## Report Formats

Use `--format` to select the report writer:

| Format | Use case |
| --- | --- |
| `text` | local human-readable summaries |
| `json` | compact automation output |
| `json-pretty` | readable JSON for review |
| `xml` | veraPDF-style machine-readable report workflows |
| `mrr` | deprecated compatibility alias for `xml` |
| `raw` | processor-style raw XML report |
| `html` | static human-readable HTML report |

For CI logs, prefer `json` plus `--redact-paths` when paths may expose private data:

```bash
pdfv validate ./documents --recursive --format json --redact-paths
```

## Exit Codes

| Code | Meaning |
| --- | --- |
| 0 | all processed files are valid |
| 1 | validation completed and at least one file is invalid |
| 2 | at least one file could not be parsed |
| 3 | at least one file is encrypted and could not be validated |
| 4 | validation is incomplete because required rules are unsupported |
| 64 | invalid CLI arguments or config |
| 70 | internal processing failure |

For batch runs, the worst category wins.

## YAML Config

CLI flags override YAML values.

```yaml
validation:
  flavour: auto
  defaultFlavour: pdfa-1b
  maxFailedAssertionsPerRule: 100
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

Run with:

```bash
pdfv validate --config pdfv.yaml ./documents --recursive
```

## Password-Protected PDFs

pdfv does not accept literal passwords on the command line. Use one of the safe source options:

```bash
printf '%s' "$PDF_PASSWORD" | pdfv validate encrypted.pdf --password-stdin
```

```bash
pdfv validate encrypted.pdf --password-file password.txt
```

```bash
PDF_PASSWORD='secret' pdfv validate encrypted.pdf --password-env PDF_PASSWORD
```

Password values are redacted from reports and `Debug` output.

## Feature and Policy Reports

Extract all supported feature families:

```bash
pdfv validate document.pdf --extract --format json
```

Extract selected families:

```bash
pdfv validate document.pdf --extract catalog,page,font --format json
```

Evaluate a bounded YAML policy over the feature report:

```bash
pdfv validate document.pdf --policy-file policy.yaml --format xml
```

Policy files use pdfv's bounded YAML policy format. Arbitrary Schematron and XSLT execution are intentionally out of scope.

## Metadata Repair

Metadata repair is conservative and never modifies inputs in place. Outputs are written under a caller-selected directory.

```bash
mkdir -p repaired
pdfv repair-metadata document.pdf --output-dir repaired --prefix fixed- --format json
```

Current repair behavior:

- valid single-flavour inputs may be copied unchanged with a `noAction` report;
- parse failures, encrypted inputs, invalid outputs, and unsupported cases are refused with structured reasons;
- failed outputs are removed and pre-existing outputs are preserved.

## veraPDF Migration Notes

pdfv supports migration-oriented aliases:

- `--defaultflavour` for `--default-flavour`
- `--recurse` for `--recursive`
- `--nonpdfext` for `--non-pdf-extension`

Important intentional differences:

- no literal `--password <text>` argument;
- metadata repair is a separate `repair-metadata` command, not a validation flag;
- policy files use bounded YAML, not arbitrary XSLT/Schematron;
- GUI, installer, ZIP input, server mode, progress display, and embedded report logs are out of scope.
