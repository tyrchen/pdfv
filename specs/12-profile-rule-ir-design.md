# 12-profile-rule-ir-design: Profiles and Bounded Rule IR

Status: draft · Owner: pdfv · Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md)

## 1. Purpose

Profiles keep validation data-driven. This subsystem loads built-in/custom profiles, parses rule tests into a bounded expression IR, indexes rules by object type, and reports unsupported rules explicitly. It does not traverse documents or format reports.

## 2. Interface

```rust
pub trait ProfileRepository {
    fn profiles_for(&self, selection: &FlavourSelection) -> Result<Vec<ValidationProfile>, PdfvError>;
}

pub trait RuleEvaluator {
    fn evaluate(&mut self, object: ModelObjectRef<'_>, rule: &Rule) -> Result<RuleOutcome, PdfvError>;
}
```

The default repository starts with a built-in profile catalog compiled into Rust data. The M4 default profile includes parser smoke checks plus executable feature-fact rules for page, font, annotation, output-intent, and content-stream model objects. A later generator converts veraPDF XML or Arlington-derived data into static Rust data. Custom profile loading uses `quick-xml 0.40.1` with byte and element caps.

## 3. Rule IR

`RuleExpr` is a closed enum:

```rust
pub enum RuleExpr {
    Bool(bool),
    Number(DecimalNumber),
    String(BoundedText),
    Property(PropertyPath),
    Unary { op: UnaryOp, expr: ExprId },
    Binary { op: BinaryOp, left: ExprId, right: ExprId },
    Call { function: BuiltinFunction, args: SmallVec<[ExprId; 4]> },
}
```

The IR supports only deterministic, side-effect-free operations needed by converted profiles. It has no file, network, clock, random, reflection, allocation-unbounded regex, or host callbacks. Evaluation has instruction and recursion budgets.

## 4. Behaviour

Profile selection follows [10-data-model.md § 5](./10-data-model.md#5-profiles-and-rules). Auto mode can return multiple compatible profiles, matching veraPDF’s ability to detect PDF/A, PDF/UA, and WTPDF flavours from XMP (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:171`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:178`). Incompatible profiles are retained as warnings rather than log-only events, improving on veraPDF’s warning-only compatibility skip (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:115`).

Unsupported expressions produce `UnsupportedRule` records with profile id, rule id, expression fragment, and reason. Unsupported rules do not mark a document compliant. A profile report with unsupported required rules has `status = Incomplete`.

Variables are initialized before traversal and updated after each object’s rule checks, preserving veraPDF’s order (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:222`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:248`).

## 5. Open risk gates

The implementation plan requires `spike-profile-expression-ir.md` before broad profile conversion. The spike must parse a representative subset of veraPDF rule expressions and prove the IR covers enough of M0/M1 rules without embedding JavaScript.

## 6. AGENTS.md binding

- Error Handling: `ProfileError` and `RuleError` are `thiserror` enums; malformed profile XML returns `Result`, not panic.
- Safety & Security: custom profiles are hostile input; XML reader has byte, depth, element-count, string, and attribute caps.
- Type Design & API: profile IDs, object type names, rule tags, and references are newtypes with fallible constructors.
- Serialization: profile metadata and unsupported rules serialize as `camelCase`.
- Testing: rstest tables for expression parsing, proptest for bounded expression trees, fixture tests for invalid XML.
- Logging & Observability: profile load emits structured spans without profile body text.
- Performance: rule indexes are built once per `Validator`; per-object lookup is by object type and supertypes.
- Documentation: public custom-profile APIs document accepted profile format and error cases.

## 7. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md)
- → Consumed by: [13-validation-engine-design.md](./13-validation-engine-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
