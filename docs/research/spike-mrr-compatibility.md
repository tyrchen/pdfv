# Spike: MRR/XML Compatibility

Status: complete · Owner: pdfv · Last updated: 2026-05-16

## Question

Should pdfv implement veraPDF-style MRR/XML report output, and if so which public format name should the CLI expose?

## Sources

- Current veraPDF CLI validation documentation: `xml` and `mrr` refer to the same report format, and `mrr` is deprecated starting with veraPDF 1.24. Source checked 2026-05-16: <https://docs.verapdf.org/cli/validation/>
- Current veraPDF CLI configuration documentation: `XML`/`MRR` is the machine-readable report setting. Source checked 2026-05-16: <https://docs.verapdf.org/cli/config/>
- Prior architecture study: [study-verapdf-validator-architecture.md](study-verapdf-validator-architecture.md)
- Reporting target: [../../specs/20-reporting-design.md](../../specs/20-reporting-design.md)

## Findings

veraPDF's user-facing machine-readable report examples use a `<report>` root containing build information, `<jobs>`, one `<job>` per input, `<item>`, `<validationReport>`, `<details>`, durations, and `<batchSummary>`. Current documentation treats `xml` as the report format users should select; `mrr` remains a historical alias.

pdfv already has stable `ValidationReport` and `BatchReport` structures with enough data for a compatibility subset:

- engine version
- input name and byte length
- per-profile compliance
- rule/check counters
- failed and passed assertion details
- unsupported rule details
- parse facts, warnings, and batch counters

The current product does not have veraPDF feature reports, repair reports, policy reports, raw XML configuration dumps, log embedding, or full release component metadata. Implementing those would invent empty or misleading XML sections. A compatibility subset is more honest and still useful for CI systems that need XML ingestion.

## Decision

Implement XML compatibility output now:

- canonical CLI format: `--format xml`
- deprecated compatibility alias: `--format mrr`
- library format: `ReportFormat::Xml`
- writer: streaming, escaped XML generated from existing report contracts

Do not implement veraPDF raw XML, HTML, policy reports, repair reports, feature reports, or log embedding in this phase.

## Follow-up Scope

Future M4 work can extend the same XML writer when richer feature facts land. Any schema-stable compatibility promise should wait until pdfv has a broader fixture matrix comparing pdfv XML against veraPDF XML on the same input corpus.
