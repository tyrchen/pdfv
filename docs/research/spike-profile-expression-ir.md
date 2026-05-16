# Spike: Profile Expression IR Coverage

Status: Done · Owner: pdfv · Date: 2026-05-15

## Question

Can representative veraPDF profile rule expressions be represented by a deterministic Rust IR without embedding JavaScript?

## Inputs Reviewed

- `vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3A.xml`
- Existing architecture study: [study-verapdf-validator-architecture.md](./study-verapdf-validator-architecture.md)
- Design target: [../../specs/12-profile-rule-ir-design.md](../../specs/12-profile-rule-ir-design.md)

## Representative Expressions

The profile XML includes these M0-relevant forms:

- Boolean and numeric comparisons: `headerOffset == 0`, `postEOFDataSize == 0`, `Length == realLength`.
- Boolean conjunction/disjunction: `streamKeywordCRLFCompliant == true && endstreamKeywordEOLCompliant == true`.
- Null checks: `lastID != null`, `F == null && FFilter == null`.
- Numeric ranges and arithmetic: `hexCount % 2 == 0`, `intValue <= 2147483647`.
- String equality against constants: `internalRepresentation == "FlateDecode"`.
- Regular-expression predicate for the PDF header.
- JavaScript collection helpers in broader rules: `split`, `filter`, `toString`, and arrow functions.

## Findings

M0 does not need a JavaScript engine. The parser-fact and basic COS rules can be represented by a closed expression IR with literals, property paths, unary and binary operators, modulo, bounded string equality, regex predicates compiled by Rust's linear-time `regex` crate, and small built-in list helpers.

Broader profile conversion needs explicit unsupported-rule reporting for JavaScript collection/filter expressions until the profile importer maps those idioms into bounded built-ins. That matches the design requirement that unsupported required rules make the profile report incomplete instead of silently compliant.

## Decisions

- Keep D4 in [../../specs/99-key-decisions.md](../../specs/99-key-decisions.md): bounded Rust IR, no JavaScript.
- Phase 3 may implement the M0 rule subset directly against this IR.
- Broad profile import remains gated on a later generator/importer task.

## Spec Impact

[../../specs/12-profile-rule-ir-design.md](../../specs/12-profile-rule-ir-design.md) already captures the required IR shape and unsupported-rule behavior. No M0 scope reduction is required.
