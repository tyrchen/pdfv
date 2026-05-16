# 16-profile-catalog-rule-parity-design: Profile Catalog and Rule Parity

Status: draft · Owner: pdfv · Depends on: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md)

## 1. Purpose

This spec turns the current bounded profile importer into a veraPDF-profile parity pipeline. It owns built-in profile catalog generation, official profile coverage, expression lowering, unsupported-rule accounting, and profile compatibility selection. It does not own the validation model properties that expressions read; those belong to [17-validation-model-parity-design.md](./17-validation-model-parity-design.md).

veraPDF loads all profile XML resources by iterating `PDFAFlavour.values()` (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/ProfileDirectoryImpl.java:154`). The vendored catalog includes PDF/A-1/2/3/4, PDF/UA, and WTPDF XML files under `vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/`. pdfv currently embeds only `PDFA-1B.xml` plus a small `pdfv-m4` profile, so catalog parity is a major gap.

## 2. Interface

```rust
pub struct GeneratedProfileCatalog {
    profiles: &'static [GeneratedProfile],
    expression_coverage: ExpressionCoverage,
    source_pin: VendorProfilePin,
}

pub trait ProfileGenerator {
    fn generate(&self, sources: ProfileSources<'_>) -> Result<GeneratedProfileCatalog, ProfileGenError>;
}

pub struct ExpressionCoverage {
    pub total_rules: u64,
    pub executable_rules: u64,
    pub unsupported_rules: Vec<UnsupportedRuleSummary>,
}
```

Generation is an offline build tool invoked by a Makefile target. Runtime profile loading never shells out and never reads vendored source paths except through explicitly configured custom profiles.

## 3. Built-in Catalog Scope

The built-in catalog target is:

- PDF/A: `PDFA-1A`, `PDFA-1B`, `PDFA-2A`, `PDFA-2B`, `PDFA-2U`, `PDFA-3A`, `PDFA-3B`, `PDFA-3U`, `PDFA-4`, `PDFA-4E`, `PDFA-4F`
- PDF/UA: `PDFUA-1`, `PDFUA-2-ISO32005`
- WTPDF: `WTPDF-1-0-Reuse`, `WTPDF-1-0-Accessibility`

Each generated profile records:

- profile id and display name
- vendor source file and commit pin
- source flavour
- rule ids, descriptions, references, object type, deferred flag
- executable `RuleExpr` or structured `UnsupportedRule`
- expression-lowering diagnostics

## 4. Expression Lowering

The rule IR remains bounded and side-effect free per [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md). Parity requires lowering the common veraPDF expression surface rather than embedding JavaScript.

Required expression features:

- boolean, number, string, and null literals
- property reads, nested paths, and dictionary direct properties
- `!`, `&&`, `||`, comparison operators
- arithmetic needed by official rules
- ternary `?:` where used in official profiles
- modulo `%` where used in official profiles
- collection functions such as `size`, `isEmpty`, `contains`, `filter`, `all`, and `exists` as bounded built-ins
- regex-like checks only through Rust `regex` with size/dfa limits
- explicit unsupported nodes for host-specific or non-deterministic constructs

The current importer rejects `?`, `%`, and `.` in expressions. This spec requires those gaps to be retired by grammar support or deliberately counted unsupported with profile-specific evidence.

## 5. Compatibility and Status Semantics

Runtime profile selection follows veraPDF's multi-flavour auto-detection model but keeps warnings structured:

- Auto mode may return multiple compatible profiles.
- Incompatible detected profiles are skipped with a warning in `ValidationReport`.
- Unsupported required rules make the profile report incomplete, never compliant.
- The CLI `profiles list` must show every built-in profile, source pin, and executable coverage percentage.

veraPDF filters incompatible detected profile versions with a warning (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:103`). pdfv must make the skip visible in JSON/XML/text reports.

## 6. Invariants

- Generated Rust data is deterministic byte-for-byte for a fixed vendor pin.
- No runtime JavaScript engine is introduced.
- Unsupported rules are first-class report data, not logs.
- Every generated public profile is selectable by CLI and library API.
- Every generated profile has at least one fixture proving it loads and validates a minimal document to `Invalid`, `Valid`, or `Incomplete` deterministically.

## 7. AGENTS.md binding

- Error Handling: generator and runtime profile errors are `thiserror` enums with source errors.
- Safety & Security: custom XML profiles remain hostile input with byte/depth/element/string caps.
- Type Design & API: profile ids, flavour ids, rule ids, object type names, and references stay newtyped.
- Testing: snapshot generated catalog metadata; fixture tests for each profile; expression parser property tests; coverage regression test fails when executable coverage decreases unexpectedly.
- Performance: rule indexes are generated or built once per profile, not per object.
- Documentation: generated coverage summary is published under docs or report examples when profiles change.

## 8. Cross-references

- ← Depends on: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md)
- → Consumed by: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md), [../docs/research/spike-profile-expression-ir.md](../docs/research/spike-profile-expression-ir.md)
