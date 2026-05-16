# 10-data-model: Public Contracts and Validation Records

Status: draft · Owner: pdfv · Depends on: [00-prd.md](./00-prd.md)

## 1. Purpose

This spec defines the stable data contracts shared by parser, profile engine, validator, reporting, and CLI. It is intentionally earlier than parser design because downstream code must not invent incompatible report or config shapes.

## 2. Public API shape

```rust
#[non_exhaustive]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, typed_builder::TypedBuilder)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationOptions {
    pub flavour: FlavourSelection,
    pub resource_limits: ResourceLimits,
    pub max_failed_assertions_per_rule: MaxDisplayedFailures,
    pub record_passed_assertions: bool,
    pub report_parse_warnings: bool,
    #[serde(skip, default)]
    pub password: Option<PasswordSecret>,
}

#[non_exhaustive]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlavourSelection {
    Auto { default: Option<ValidationFlavour> },
    Explicit { flavour: ValidationFlavour },
    CustomProfile { profile_path: std::path::PathBuf },
}
```

All public structs with more than five fields use `typed-builder` per AGENTS.md Type Design & API. All fallible constructors return `Result<T, PdfvError>` with `thiserror` library errors. CLI layers may wrap with `anyhow::Context`.

Password-bearing options follow [14-password-decryption-design.md](./14-password-decryption-design.md): password values use redacted secret wrappers, are never serialized/deserialized, and are exposed only inside the decryption module.

## 3. Core records

`ValidationReport`:

- `engineVersion: String`
- `source: InputSummary`
- `status: ValidationStatus`
- `flavours: Vec<ValidationFlavour>`
- `profileReports: Vec<ProfileReport>`
- `parseFacts: Vec<ParseFact>`
- `warnings: Vec<ValidationWarning>`
- `taskDurations: Vec<TaskDuration>`

`ProfileReport`:

- `profile: ProfileIdentity`
- `isCompliant: bool`
- `checksExecuted: u64`
- `rulesExecuted: u64`
- `failedRules: u64`
- `failedAssertions: Vec<Assertion>`
- `passedAssertions: Vec<Assertion>` only when configured
- `unsupportedRules: Vec<UnsupportedRule>`

`Assertion`:

- `ordinal: NonZeroU64`
- `ruleId: RuleId`
- `status: AssertionStatus`
- `description: String`
- `location: ObjectLocation`
- `objectContext: Option<String>`
- `message: Option<String>`
- `errorArguments: Vec<ErrorArgument>`

`BatchReport`:

- `items: Vec<ValidationReport>`
- `summary: BatchSummary`
- `warnings: Vec<ValidationWarning>`

## 4. Parser facts

Parser facts are first-class, queryable validation inputs, not logs. veraPDF’s parser records facts such as header offset, stream EOL compliance, real stream size, and xref compliance; rules can assert on these facts in the Java implementation (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/PDFParser.java:116`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:190`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:232`; see research memo § Hot path).

```rust
#[non_exhaustive]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ParseFact {
    Header { offset: u64, version: PdfVersion, hadLeadingBytes: bool },
    PostEofData { bytes: u64 },
    Xref { section: ObjectLocation, fact: XrefFact },
    Stream { object: ObjectKey, fact: StreamFact },
    Encryption { encrypted: bool, handler: Option<String> },
}
```

Facts are bounded by `ResourceLimits.max_parse_facts`. If the cap is reached, validation adds `ValidationWarning::ParseFactCapReached` and continues where safe.

## 5. Profiles and rules

`ValidationProfile` is immutable after loading. Built-in profiles are compiled into static data in a later profile generation phase; custom profiles load from bounded XML.

```rust
pub struct ValidationProfile {
    pub identity: ProfileIdentity,
    pub flavour: ValidationFlavour,
    pub rules: Vec<Rule>,
    pub variables: Vec<Variable>,
}

pub struct Rule {
    pub id: RuleId,
    pub object_type: ObjectTypeName,
    pub deferred: bool,
    pub tags: Vec<RuleTag>,
    pub description: BoundedText,
    pub test: RuleExpr,
    pub error: ErrorTemplate,
    pub references: Vec<SpecReference>,
}
```

This mirrors veraPDF’s `ValidationProfile` and `Rule` contracts: profiles expose rules and variables by object name, and rules carry object, deferred flag, description, test, error details, and references (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/ValidationProfile.java:60`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/ValidationProfile.java:78`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/Rule.java:49`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/Rule.java:70`).

## 6. Error model

`PdfvError` is a `thiserror` enum with variants:

- `Io { path: Option<PathBuf>, source: io::Error }`
- `Parse(ParseError)`
- `Profile(ProfileError)`
- `Validation(ValidationError)`
- `Report(ReportError)`
- `Configuration(ConfigError)`

No public library error uses `anyhow`. No external-input path panics. Absence is `Option` only when it is semantically optional; parse failure is `Result` per AGENTS.md Error Handling.

## 7. Invariants

| Invariant | Enforcement |
| --- | --- |
| Public JSON is `camelCase`. | Derive serde with `rename_all = "camelCase"` and snapshot tests. |
| Public structs are `Debug`; secrets are redacted. | Derives plus explicit tests for password-bearing types. |
| All externally supplied strings have byte caps. | `BoundedText`, `ObjectTypeName`, `RuleTag`, `ProfileId` newtypes. |
| All object offsets, lengths, counts, and arithmetic are checked. | Parser uses `checked_*` and returns `ParseError::LimitExceeded` or `ParseError::ArithmeticOverflow`. |
| Report detail caps never hide counters. | Summary counters are complete; `failedAssertions` is bounded. |

## 8. Cross-references

- ← Depends on: [00-prd.md](./00-prd.md)
- → Consumed by: [11-parser-core-design.md](./11-parser-core-design.md), [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [14-password-decryption-design.md](./14-password-decryption-design.md), [20-reporting-design.md](./20-reporting-design.md), [50-cli-design.md](./50-cli-design.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
