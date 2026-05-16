# Spike: Bounded Feature Policy Language

Status: complete · Owner: pdfv · Last updated: 2026-05-16

## Question

What policy format should Phase 16 implement without exposing filesystem, network, XSLT, regex, or raw-PDF access?

## Findings

Phase 16 policy execution must consume `FeatureReport`, not PDF bytes. That keeps the boundary explicit:

```text
PDF bytes -> ParsedDocument -> ModelGraph -> FeatureReport -> PolicyReport
```

veraPDF policy compatibility can involve XML-oriented workflows, but executing arbitrary XSLT or Schematron over attacker-influenced feature XML would add a separate sandboxing problem. The first pdfv policy surface therefore uses a data-only YAML shape that deserializes into typed Rust records and evaluates fixed operators over extracted feature fields.

## Decision

Implement a bounded internal YAML policy format:

- `rules` is required and capped in code.
- Each rule names `id`, `description`, `family`, `field`, `operator`, and optional typed `value`.
- Operators are fixed: `exists`, `absent`, `equals`, `notEquals`, `min`, and `max`.
- Evaluation reads only `FeatureReport` objects and cannot access raw PDF bytes, filesystem, network, clock, subprocesses, regex engines, or dynamic code.
- Policy results merge into JSON/XML reports as `policyReport`.

This is intentionally not a Schematron/XSLT compatibility layer. A future phase can add that only with a dedicated security design.

## Example

```yaml
name: no-metadata-required
rules:
  - id: catalog-metadata-absent
    description: Catalog metadata is absent
    family: catalog
    field: hasMetadata
    operator: equals
    value:
      type: bool
      value: false
```
