# 19-verapdf-product-surface-parity-design: veraPDF Product Surface Parity

Status: draft · Owner: pdfv · Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [20-reporting-design.md](./20-reporting-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [50-cli-design.md](./50-cli-design.md)

## 1. Purpose

This spec describes the optional product surfaces needed to approach veraPDF parity beyond validation: feature extraction, policy reports, metadata repair, raw/HTML reports, richer CLI flags, and compatibility packaging. It exists separately because these are product features, not prerequisites for core validation correctness.

veraPDF's processor executes validation, metadata repair, and feature extraction tasks (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/TaskType.java:26`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorImpl.java:118`). Its report factory supports text, raw, XML/MRR, HTML, and JSON (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorFactory.java:128`). pdfv currently implements validation plus JSON/text/XML output.

## 2. Product Surfaces

| Surface | Parity target | Scope decision |
| --- | --- | --- |
| Feature extraction | Extract machine-readable document feature reports for policy and inventory workflows. | In scope after model parity; read-only. |
| Policy reports | Apply policy rules over feature reports and merge into XML/JSON reports. | In scope after feature extraction; policy language requires separate spike. |
| Metadata repair | Write repaired PDF metadata based on validation findings. | Separate opt-in command; never mixed into default validation. |
| Raw report | Serialize processor config, item details, validation, feature, and repair results. | In scope for migration workflows. |
| HTML report | Transform XML/JSON report into human-readable HTML. | In scope after XML parity; static assets only. |
| CLI compatibility flags | `--extract`, `--fixmetadata`, policy file, default flavour, verbose/detail controls, non-PDF extension selection. | Add only when backing core feature exists. |
| Packaging | Single binary and library crate remain primary. | GUI, installer, server mode, and auto-updater are out of scope until separately specified. |

## 3. Feature Extraction

Feature extraction uses the model graph from [17-validation-model-parity-design.md](./17-validation-model-parity-design.md) plus the semantic subsystems in [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), and [23-structure-accessibility-design.md](./23-structure-accessibility-design.md). It produces a `FeatureReport` from selected feature families:

- document metadata
- pages and page resources
- fonts and CMaps
- images and XObjects
- annotations and forms
- actions and embedded files
- structure/accessibility objects
- color spaces and output intents
- signatures and permissions facts

Feature extraction is read-only and uses the same parser/model resource limits as validation. Plugins are not in the first feature phase; a plugin ABI would need its own security design.

## 4. Policy Reports

Policy reports consume feature reports, not raw PDFs. This preserves a clear boundary:

```text
PDF bytes -> ParsedDocument -> ModelGraph -> FeatureReport -> PolicyReport
```

The first policy phase supports a bounded internal rule format. Schematron/XSL compatibility is a research item because running arbitrary XSLT over attacker-influenced feature XML has a different security profile.

## 5. Metadata Repair

Metadata repair is an explicit command:

```text
pdfv repair-metadata <paths>... --output-dir <dir> [--prefix <prefix>]
```

Rules:

- Never modify input files in place.
- Write repaired files to a caller-selected output directory.
- Preserve original bytes where possible and rewrite only metadata objects needed for the fix.
- Require successful parse and exactly one selected validation flavour.
- Produce a `RepairReport` with actions taken and refusal reasons.
- Never run for encrypted inputs unless decryption and safe rewrite support are explicitly implemented.

veraPDF deletes unsuccessful repair outputs when repair status is not successful (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorImpl.java:298`). pdfv must use atomic temp-file writes and remove failed outputs.

## 6. Report Formats

Extend [20-reporting-design.md](./20-reporting-design.md):

- `ReportFormat::RawXml` for raw processor-style results.
- `ReportFormat::Html` generated from XML/JSON without external network assets.
- Feature and repair sections in JSON/XML/Raw where present.
- Logs remain opt-in and redacted; default reports do not embed tracing logs.

## 7. CLI Extensions

Extensions land only after backing library APIs exist:

```text
pdfv validate --default-flavour <flavour>
pdfv validate --extract <features>
pdfv validate --policy-file <path>
pdfv validate --format <json|json-pretty|text|xml|mrr|raw|html>
pdfv repair-metadata <paths>... --output-dir <dir> --prefix <prefix>
```

The existing no-literal-password policy remains. veraPDF accepts `--password <text>` (`vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/commands/VeraCliArgParser.java:177`); pdfv deliberately does not because it conflicts with project secret-handling rules.

## 8. Non-goals

- GUI parity.
- Java installer parity.
- Auto-update checks.
- Server mode.
- Plugin loading from arbitrary dynamic libraries or jars.
- Rendering or visual diffing.

## 9. AGENTS.md binding

- Error Handling: feature, policy, and repair errors are distinct `thiserror` enums and reportable per item in batch runs.
- Safety & Security: repair writes use canonicalized output directories and atomic writes; policy execution cannot access filesystem/network unless separately specified.
- Cryptography & Secrets: no report surface includes passwords, keys, decrypted content bytes, or secret source values.
- Testing: golden reports for raw/HTML, repair refusal tests, atomic-write tests, policy fixtures, and CLI integration tests.
- Performance: feature extraction can be more expensive than validation but must respect the same file/stream/model caps.
- Documentation: parity docs state which veraPDF CLI flags are supported, intentionally different, or out of scope.

## 10. Cross-references

- ← Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [20-reporting-design.md](./20-reporting-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [50-cli-design.md](./50-cli-design.md)
- → Consumed by: [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md), [../docs/research/spike-mrr-compatibility.md](../docs/research/spike-mrr-compatibility.md)
