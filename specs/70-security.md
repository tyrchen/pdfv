# 70-security: Threat Model and Safety Rules

Status: draft · Owner: pdfv · Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md)

## 1. Threat model

Inputs are hostile: PDF bytes, file names, CLI args, YAML config, custom profiles, environment variables, and output paths. Attack goals include CPU exhaustion, memory exhaustion, decompression bombs, parser panic, path traversal, misleading logs, and report injection.

## 2. Binding rules

- `#![forbid(unsafe_code)]` in every crate. No unsafe blocks in tests.
- No `unwrap`, `expect`, `panic`, unchecked indexing, or `unreachable` reachable from external input.
- Every external string has a byte cap and a charset policy where it becomes an identifier.
- Every collection parsed from input has an element cap.
- Every integer derived from input has range validation and checked arithmetic.
- Every decompressor is streaming and byte-counted.
- Every parser recursion is replaced with explicit stacks and depth counters.
- Content-stream operator parsing has explicit operator, operand-count, operand-byte, graphics-state-depth, and Type 3 charproc limits.
- Resource inheritance, structure tree traversal, parent tree traversal, and accessibility graph reconstruction have explicit node/depth/edge caps and cycle detection.
- Every CLI output path is validated and opened with normal Rust APIs; no shelling out.
- Raw PDF bytes are never logged or included in error messages.

## 3. Resource limits

Resource limits are mandatory and part of `ValidationOptions`. Defaults are conservative and documented. Config may raise them, but validation still requires upper hard caps compiled into constants for obviously dangerous values.

## 4. Custom profile safety

Custom profile XML is hostile input. The loader enforces file size, XML depth, element count, attribute count, string byte caps, and known-schema validation. Rule expressions compile to the bounded IR in [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md); no JavaScript, regex backtracking engines, file access, network access, or clock access.

## 5. Passwords and encrypted PDFs

M0 detects encryption but does not decrypt. M4 password/decryption support is scoped by [14-password-decryption-design.md](./14-password-decryption-design.md).

Binding rules:

- Password values use redacted secret wrappers and are never serialized in configs or reports.
- The CLI does not accept literal password arguments; supported sources are stdin, file, or environment variable indirection.
- Password comparisons use constant-time comparison where candidate values are compared with `/U` or `/O`-derived bytes.
- MD5 and RC4 are allowed only for PDF Standard security handler compatibility for revisions 2-4; SHA-2 and AES-256 are allowed only for future Standard security handler revisions 5-6. None of these are general-purpose cryptographic utilities.
- Decrypted strings and streams remain hostile input and are subject to byte-counted limits before validation or downstream decoding.
- Invalid AES-256 `/Perms` validation is treated as encrypted/unsupported rather than as a recoverable warning until a compatibility corpus proves looser behavior is necessary.

## 6. Verification gates

- Parser fuzz target for arbitrary byte input.
- Clippy boundary lint run with `-W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic` for parser/profile modules.
- Corpus tests for truncated files, recursive objects, giant arrays, giant names, bad lengths, decompression bombs, and invalid UTF-8.
- Operator/resource/accessibility tests for huge operand lists, resource cycles, parent-tree cycles, malformed MCID references, and over-deep structure trees.
- `cargo audit` and `cargo deny check` required before release.
- Password-bearing type `Debug` redaction test.
- Encrypted fixture matrix covering missing password, wrong password, supported user/owner password, unsupported revision, malformed `/Encrypt`, and future R5/R6 AESV3 `/Perms` tampering.

## 7. Cross-references

- ← Depends on: [10-data-model.md](./10-data-model.md), [11-parser-core-design.md](./11-parser-core-design.md)
- → Constrains: [14-password-decryption-design.md](./14-password-decryption-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [91-impl-plan.md](./91-impl-plan.md)
