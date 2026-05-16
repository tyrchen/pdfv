# 14-password-decryption-design: Passwords and PDF Decryption

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-16 · Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [50-cli-design.md](./50-cli-design.md), [70-security.md](./70-security.md)

## 1. Purpose

This spec scopes M4 password/decryption support. The goal is to validate password-protected PDFs when the caller supplies the correct password, while keeping secrets out of reports, logs, configs, debug output, and process lists.

This subsystem owns PDF encryption dictionary parsing, password authentication, object/string/stream decryption, decrypted-byte resource limits, and public password input contracts. It does not own password cracking, password recovery, certificate/public-key security handlers, digital signatures, PDF repair, or changing PDF permissions.

## 2. Scope

M4 password support lands in two explicit slices:

| Slice | Included | Excluded |
| --- | --- | --- |
| Phase 9 | Standard security handler revisions 2-4; `/Filter /Standard`; `/V` 1, 2, and 4; RC4; AESV2; owner/user password authentication; object strings; streams; `/EncryptMetadata`; `/Crypt` identity handling. | AES-256 revisions 5-6; public-key security handlers; non-standard filters; editing permissions; writing encrypted PDFs. |
| Future risk gate | Standard security handler revisions 5-6; AESV3; PDF 2.0 hash algorithm; `/Perms` validation. | Password recovery, DRM enforcement, public-key encryption. |

veraPDF follows the same high-level shape: it reads the `/Encrypt` dictionary, only decrypts through the Standard security handler, checks the password before setting a document security handler, and decrypts strings/streams through object-key-aware filters (`vendors/veraPDF-parser/src/main/java/org/verapdf/io/Reader.java:155`, `vendors/veraPDF-parser/src/main/java/org/verapdf/io/Reader.java:175`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/encryption/StandardSecurityHandler.java:76`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/encryption/StandardSecurityHandler.java:151`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/encryption/StandardSecurityHandler.java:174`). pdfv differs by keeping password state explicit in `ValidationOptions`/`ValidationSession` instead of a thread-local (`vendors/veraPDF-parser/src/main/java/org/verapdf/tools/StaticResources.java:59`).

## 3. Public API

`ValidationOptions` gains an optional redacted password source:

```rust
#[non_exhaustive]
#[derive(Clone)]
pub struct ValidationOptions {
    #[serde(skip, default)]
    pub password: Option<PasswordSecret>,
    // existing fields...
}

#[derive(Clone)]
pub struct PasswordSecret(secrecy::SecretString);
```

`PasswordSecret`:

- is never `Serialize` or `Deserialize`;
- implements `Debug` manually as redacted;
- has fallible constructors enforcing a byte cap;
- exposes bytes only inside the decryption module through `ExposeSecret`;
- accepts UTF-8 input from the public API and converts per security-handler revision at authentication time.

The parser receives password state through an explicit parse context:

```rust
pub struct Parser {
    limits: ResourceLimits,
}

pub struct ParseOptions<'a> {
    pub password: Option<&'a PasswordSecret>,
}

impl Parser {
    pub fn parse_with_options<R: PdfSource>(
        &self,
        source: R,
        options: ParseOptions<'_>,
    ) -> Result<ParsedDocument, PdfvError>;
}
```

`Parser::parse` remains available and behaves as today: encrypted documents are detected but not decrypted.

## 4. CLI and Config

CLI password input is intentionally not `--password <text>` because command-line arguments are commonly visible through shell history, process listings, and CI logs.

`pdfv validate` gains:

```text
    --password-stdin
    --password-file <path>
    --password-env <ENV_VAR>
```

Rules:

- exactly zero or one password source is allowed;
- password values are trimmed only for one trailing line ending from stdin/file to support normal secret injection, not generally sanitized;
- `--password-file` refuses directories and files larger than the configured password byte cap;
- `--password-env` reads the named variable and never serializes its value into config/report output;
- YAML config may set the password source kind/path/env-var name, but never a literal password value.

Batch validation uses the same password for every input in the first implementation. Per-file password maps are deferred because they require new redaction, matching, and report UX rules.

## 5. Encryption Model

The parser materializes an `EncryptionDictionary` from `/Encrypt` before decrypting encrypted objects:

```rust
#[derive(Clone, Debug)]
pub struct EncryptionDictionary {
    pub filter: EncryptionFilter,
    pub sub_filter: Option<Identifier>,
    pub version: EncryptionVersion,
    pub revision: SecurityRevision,
    pub key_length_bits: KeyLengthBits,
    pub owner_key: Vec<u8>,
    pub user_key: Vec<u8>,
    pub permissions: PermissionsBits,
    pub document_id: Vec<u8>,
    pub encrypt_metadata: bool,
    pub crypt_filters: CryptFilterSet,
    pub stream_filter: CryptFilterName,
    pub string_filter: CryptFilterName,
}
```

All primitive wrappers are fallible:

- `SecurityRevision` accepts only 2, 3, and 4 in Phase 9.
- `KeyLengthBits` accepts the PDF-defined range for the selected revision and rounds nothing.
- `PermissionsBits` stores the signed 32-bit `/P` value as bytes for key derivation and exposes permission booleans separately.
- `CryptFilterSet` accepts only direct `/CF` dictionaries in Phase 9.

Unsupported encryption dictionaries produce structured parse errors or encrypted reports:

| Condition | Outcome |
| --- | --- |
| Encrypted PDF, no password supplied | `ValidationStatus::Encrypted`, no validation traversal. |
| Password supplied but wrong | `ValidationStatus::Encrypted` plus a bounded warning `incorrect password`; exit code remains 3. |
| Unsupported security handler/filter/revision | `ValidationStatus::Encrypted` plus a bounded warning naming the unsupported handler/revision. |
| Malformed encryption dictionary | `ValidationStatus::ParseFailed` if required fields are structurally invalid. |
| Correct password and supported algorithm | decrypted `ParsedDocument`; normal validation status. |

## 6. Algorithms

Phase 9 implements PDF Standard security handler revisions 2-4:

- password padding and truncation to 32 bytes for revisions 2-4;
- Algorithm 2 file encryption key derivation with MD5, `/O`, `/P`, first trailer ID, and `/EncryptMetadata`;
- Algorithm 4/5 user password authentication for revisions 2-4;
- owner password attempt followed by user password attempt so either valid password works;
- object-specific key derivation using object number and generation;
- RC4 decryption for `/V` 1/2 and crypt filters with `/CFM /V2`;
- AES-CBC decryption for `/CFM /AESV2`, including IV handling and padding validation;
- metadata stream handling when `/EncryptMetadata false`;
- `/Crypt` filter identity handling consistent with veraPDF's `decryptRequired` logic (`vendors/veraPDF-parser/src/main/java/org/verapdf/pd/encryption/StandardSecurityHandler.java:191`).

Phase 9 does not implement revisions 5-6. veraPDF carries a separate implementation for revision 5/6 key retrieval and revision 6 iterative hashing (`vendors/veraPDF-parser/src/main/java/org/verapdf/tools/EncryptionToolsRevision5_6.java:40`, `vendors/veraPDF-parser/src/main/java/org/verapdf/tools/EncryptionToolsRevision5_6.java:96`). pdfv requires a dedicated spike before accepting AES-256 support because it changes password encoding, hash selection, `/OE`/`/UE` handling, and `/Perms` validation.

## 7. Dependency Candidates

Versions below were checked on 2026-05-16 against current docs/crate metadata:

| Crate | Version | Use |
| --- | --- | --- |
| `secrecy` | 0.10.3 | Redacted password storage. |
| `subtle` | 2.6.1 | Constant-time comparison of candidate `/U` values. |
| `md-5` | 0.11.0 | MD5 for PDF Standard security handler revisions 2-4. |
| `rc4` | 0.2.0 | RC4 object/string/stream decryption for revisions 2-4. |
| `aes` | 0.9.0 | AES block cipher for AESV2/AES-256 candidates. |
| `cbc` | 0.2.0 | AES-CBC mode. |
| `cipher` | 0.5.1 | Shared block/stream cipher traits. |
| `sha2` | 0.11.0 | Future revision 5-6 spike. Not required for Phase 9 unless the spike lands in the same commit. |
| `pbkdf2` | 0.13.0 | Not part of PDF Standard revisions 2-6; only add if a future fixture proves a supported handler needs it. |

Every new dependency must pass `cargo audit` and `cargo deny check`; RC4 and MD5 are allowed only inside the PDF compatibility module because PDF revisions 2-4 require them for decryption of existing files. They must not be exposed as general-purpose crypto utilities.

## 8. Resource Limits

Add resource limits:

- `max_password_bytes`, default 1024, hard cap 4096;
- `max_decrypted_string_bytes`, default equal to `max_string_bytes`;
- `max_decrypted_stream_bytes`, default equal to `max_stream_decode_bytes`;
- `max_encryption_dict_entries`, default 64.

Decryption is streaming for streams and byte-counted before downstream filters. No decrypted stream may be read to unbounded memory just to authenticate or validate. String decryption is bounded by `max_decrypted_string_bytes`.

## 9. Security and Observability

AGENTS.md bindings:

- Error Handling: `EncryptionError` is a `thiserror` enum under `PdfvError::Parse` or a dedicated `PdfvError::Encryption` variant if implementation pressure justifies it.
- Safety & Security: no `unsafe`, no panics on malformed encrypted PDFs, constant-time comparisons for password checks, no raw encrypted/decrypted bytes in errors.
- Type Design & API: passwords are `PasswordSecret`, not `String`; security handler revisions, key lengths, and crypt filter methods are enums/newtypes.
- Serialization: password values are never serialized; encryption parse facts omit keys and password-derived values.
- Testing: fixture matrix covers no-password, wrong-password, user-password, owner-password, RC4, AESV2, metadata-unencrypted, unsupported revision 5/6, unsupported public-key handler, malformed `/Encrypt`, and batch exit aggregation.
- Logging & Observability: spans may include encryption revision and algorithm names, never password, `/O`, `/U`, file key, object key material, or decrypted content.
- Performance: encrypted PDFs must meet the same parser budgets plus explicit decrypted-byte caps.
- Documentation: public APIs document which revisions are supported and why MD5/RC4 appear in dependencies.

## 10. Reporting

No report includes password values or password source names.

`ParseFact::Encryption` may be extended with safe metadata:

- `encrypted: bool`
- `handler: Option<String>`
- `version: Option<u8>`
- `revision: Option<u8>`
- `algorithm: Option<String>`
- `decrypted: bool`

Warnings use bounded, non-secret messages such as `incorrect password`, `unsupported encryption revision 6`, or `unsupported public-key security handler`.

## 11. Verification

Phase 9 is complete only when:

- library validation succeeds on at least one RC4-encrypted fixture with the user password;
- library validation succeeds on at least one AESV2-encrypted fixture with the owner password;
- wrong or missing passwords return `ValidationStatus::Encrypted` and CLI exit code 3;
- unsupported revision 5/6 fixtures return encrypted/unsupported, not parse panic;
- string and stream decryption are both covered;
- malformed encryption dictionaries are fuzzed or corpus-tested;
- `Debug` for every password-bearing type is redacted by test;
- strict clippy boundary lints, `cargo audit`, and `cargo deny check` pass.

## 12. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md), [13-validation-engine-design.md](./13-validation-engine-design.md), [50-cli-design.md](./50-cli-design.md), [70-security.md](./70-security.md)
- → Consumed by: [20-reporting-design.md](./20-reporting-design.md), [72-testing-strategy.md](./72-testing-strategy.md), [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md)
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
