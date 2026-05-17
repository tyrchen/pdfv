# 20-reporting-design: Reports and Output Formats

Status: draft · Owner: pdfv · Depends on: [10-data-model.md](./10-data-model.md), [13-validation-engine-design.md](./13-validation-engine-design.md)

## 1. Purpose

Reporting turns `ValidationReport` and `BatchReport` into stable user-facing output. It does not validate, parse, or decide CLI exit codes. JSON is the stable Rust/CI machine contract; XML is a bounded compatibility report for organizations migrating from veraPDF workflows; text is a human-readable summary.

## 2. Interface

```rust
pub enum ReportFormat {
    Json,
    JsonPretty,
    Text,
    Xml,
    RawXml,
    Html,
}

pub trait ReportWriter {
    fn write_report<W: Write>(&self, report: &ValidationReport, out: W) -> Result<(), PdfvError>;
    fn write_batch<W: Write>(&self, report: &BatchReport, out: W) -> Result<(), PdfvError>;
}
```

## 3. JSON contract

JSON is stable once M0 exits. Fields are `camelCase`, unknown enum variants are handled with semver discipline, and omitted fields only use `skip_serializing_if = "Option::is_none"` where `null` would be semantically noisy. Snapshot tests pin canonical examples.

M0 JSON includes:

- top-level status and compliance
- source summary
- detected/selected flavours
- profile reports
- failed assertion details
- parse facts and warnings
- duration summary

Password/decryption support does not add any password value, password source, file key, `/O`, `/U`, `/OE`, `/UE`, or decrypted bytes to JSON.

## 4. Text contract

Text output is concise:

```text
invoice.pdf: invalid
profiles: pdfa-1b
checks: 184 passed, 3 failed, 0 unsupported
first failures:
  6.2.4-1 at root/catalog[0]/metadata[0]: XMP metadata is missing required field
```

Text output is not parsed by automation. Automation uses JSON.

## 5. XML compatibility contract

XML output is a compatibility subset of veraPDF's machine-readable report shape. The canonical CLI spelling is `--format xml`; `mrr` remains a deprecated compatibility alias because current veraPDF documentation states that `xml` and `mrr` refer to the same report format and that `mrr` is deprecated starting with veraPDF 1.24.

The XML writer streams from `ValidationReport`/`BatchReport` and includes:

- `<report>` root with pdfv build information
- `<jobs><job>` entries for each input
- `<item>` name and optional size
- `<validationReport profileName statement isCompliant>`
- `<details passedRules failedRules passedChecks failedChecks unsupportedRules>`
- bounded failed/passed checks, unsupported rules, parse facts, warnings, and batch summary

The XML compatibility surface is intentionally report-only. Raw XML and HTML are separate Phase 21 report formats over the same bounded report data; they include validation, feature, policy, and repair sections where present. Log embedding remains out of scope.

XML may include safe encryption metadata exposed through parse facts, such as handler/revision/decrypted status, but never password material or cryptographic keys.

## 6. Batch summaries

Batch summaries track total files, valid, invalid, parse failures, encrypted, incomplete, internal errors, elapsed time, and worst exit category. They are computed from item reports and do not re-run validation.

## 7. AGENTS.md binding

- Error Handling: report serialization failures return `ReportError`.
- Safety & Security: never include raw PDF bytes in output; file paths are displayed as provided by CLI unless `--redact-paths` is active.
- Cryptography & Secrets: reports must not include password values, password source names, file encryption keys, `/O`, `/U`, `/OE`, `/UE`, or decrypted stream/string bytes.
- Serialization: serde derives use `camelCase`; snapshot tests cover JSON; XML writer escapes attributes/text and never emits raw PDF bytes.
- Testing: JSON snapshot tests, text golden tests, batch summary unit tests.
- Logging & Observability: report writing itself does not log report contents.
- Performance: streaming writers avoid building duplicate large strings.
- Documentation: output examples live in CLI and library docs.

## 8. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [13-validation-engine-design.md](./13-validation-engine-design.md)
- → Consumed by: [50-cli-design.md](./50-cli-design.md)
- ↔ Related research: veraPDF report handlers in `ProcessorFactory` choose text/raw/XML/HTML/JSON by format (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorFactory.java:128`).
- ↔ Related research: [../docs/research/spike-mrr-compatibility.md](../docs/research/spike-mrr-compatibility.md)
- ↔ Related design: [14-password-decryption-design.md](./14-password-decryption-design.md)
