# 13-validation-engine-design: Validation Session and Model Graph

Status: draft · Owner: pdfv · Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md)

## 1. Purpose

The validation engine owns the end-to-end library workflow: parse input, select profiles, build a validation model graph, run rules, track counters, and return `ValidationReport`. It is the Rust replacement for veraPDF’s processor/foundry/base-validator path.

## 2. Interface

```rust
pub struct Validator {
    options: ValidationOptions,
    profiles: Arc<dyn ProfileRepository + Send + Sync>,
}

impl Validator {
    pub fn new(options: ValidationOptions) -> Result<Self, PdfvError>;
    pub fn validate_path(&self, path: impl AsRef<Path>) -> Result<ValidationReport, PdfvError>;
    pub fn validate_reader<R: Read + Seek>(&self, source: R, name: InputName) -> Result<ValidationReport, PdfvError>;
}

pub struct ValidationSession {
    caches: SessionCaches,
    limits: ResourceLimits,
    evaluator: DefaultRuleEvaluator,
}
```

`ValidationSession` is explicit state. No `ThreadLocal`, static mutable cache, global foundry registry, or hidden current document is allowed. This deliberately diverges from veraPDF’s thread-local state (`vendors/veraPDF-parser/src/main/java/org/verapdf/tools/StaticResources.java:50`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/containers/StaticContainers.java:37`).

## 3. Model object contract

```rust
pub trait ModelObject {
    fn id(&self) -> Option<ObjectIdentity>;
    fn object_type(&self) -> ObjectTypeName;
    fn super_types(&self) -> &[ObjectTypeName];
    fn extra_context(&self) -> Option<&str>;
    fn property(&self, name: PropertyName) -> Result<ModelValue, PdfvError>;
    fn links(&self) -> &[LinkName];
    fn linked_objects(&self, link: LinkName, session: &mut ValidationSession)
        -> Result<Vec<ModelObjectRef<'_>>, PdfvError>;
}
```

Model wrappers adapt parsed COS/PD data into validation object types such as document, page, catalog, metadata, stream, annotation, font, and content operation. They are lazy: linked objects are materialized when traversal reaches the link.

## 4. Traversal algorithm

The engine uses an explicit stack:

1. Push root object with context `root`.
2. Pop object and context.
3. Execute non-deferred rules for concrete type and supertypes.
4. Update variables for concrete type and supertypes.
5. Resolve links and push linked objects in reverse order.
6. After stack drains, execute deferred rules against saved object/context pairs.
7. Build one `ProfileReport` per profile.

This mirrors veraPDF’s `BaseValidator` order: stack root, `checkNext`, `checkAllRules`, `updateVariables`, `addAllLinkedObjects`, then deferred rules (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:173`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:190`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:242`).

## 5. Diagnostics

Failed assertions include rule id, profile, deterministic object path, optional object context, bounded error message, and bounded error arguments. Report counters are complete even when assertion detail caps are reached, following veraPDF’s counter/detail split (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:381`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:383`).

## 6. Behaviour

Parse failures return a `ValidationReport` with `status = ParseFailed` when enough source metadata is available; fatal I/O/configuration failures return `Err(PdfvError)`. Encrypted PDFs return `status = Encrypted` unless password support is active.

The engine supports bounded batch validation by running one `ValidationSession` per file. Shared profile repositories are immutable and `Arc` backed. Per-file caches are not shared.

## 7. AGENTS.md binding

- Error Handling: domain-specific `ValidationError`; `Validator` public methods return `Result`.
- Async & Concurrency: core validation is synchronous CPU/I/O-bound. CLI parallelism wraps it in bounded worker tasks; no async trait needed in core.
- Type Design & API: `ValidationSession` owns all mutable state; no illegal state such as zero max failures.
- Safety & Security: hostile PDFs cannot cause recursion overflow because traversal is iterative and bounded.
- Serialization: all report objects derive serde with `camelCase`.
- Testing: deterministic traversal tests pin context paths, deferred order, visited-set behaviour, and cap behaviour.
- Logging & Observability: validation spans include file name hash/path display policy, profile id, object type, and counts, not raw PDF content.
- Performance: rule indexes avoid scanning all rules per object; linked object materialization is lazy.
- Documentation: library entrypoints include examples and `# Errors` sections.

## 8. Cross-references

- ← Depends on: [11-parser-core-design.md](./11-parser-core-design.md), [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md)
- → Consumed by: [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)

