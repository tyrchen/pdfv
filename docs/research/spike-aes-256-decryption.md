# Spike: AES-256 PDF Decryption Risk Gate

Status: accepted · Owner: pdfv · Last updated: 2026-05-16

## 1. Question

Can pdfv implement Standard security handler revisions 5 and 6 without changing
the public password model, parser ownership model, or resource-limit strategy
landed for revisions 2 through 4?

Answer: yes, with a separate implementation phase. The risk gate is cleared for
an implementation-ready design, but AES-256 support should not be folded into
Phase 9 because the password validation path, dictionary shape, and object-key
derivation differ materially from revisions 2 through 4.

## 2. Sources Reviewed

- veraPDF revision 5/6 implementation:
  `vendors/veraPDF-parser/src/main/java/org/verapdf/tools/EncryptionToolsRevision5_6.java`.
- qpdf encryption implementation:
  `https://github.com/qpdf/qpdf/blob/main/libqpdf/QPDF_encryption.cc`.
- qpdf encryption documentation:
  `https://qpdf.readthedocs.io/en/latest/encryption.html`.
- PDF 2.0 errata for clause 7.6:
  `https://pdf-issues.pdfa.org/32000-2-2020/clause07.html`.
- Current RustCrypto crate metadata checked through `cargo info` and docs.rs:
  `aes` 0.9.0, `cbc` 0.2.0, `cipher` 0.5.1, `sha2` 0.11.0,
  optional `ecb` 0.2.0, and `unicode-normalization` 0.1.25.

## 3. Findings

### 3.1 Dictionary shape

Revisions 5 and 6 require `/Filter /Standard`, `/V 5`, `/R 5` or `/R 6`,
`/Length 256`, 48-byte `/O`, 48-byte `/U`, 32-byte `/OE`, 32-byte `/UE`, and a
16-byte `/Perms` string. The standard crypt filter is `/StdCF` with
`/CFM /AESV3`. PDF 2.0 deprecates earlier Standard security handler revisions
and requires AESV3 for revision 6 crypt filters.

qpdf pads short `/O`, `/U`, `/OE`, `/UE`, and `/Perms` values internally, but
pdfv should reject short values as malformed. The existing Phase 9 approach
already rejects structurally invalid encryption dictionaries instead of
repairing them, and that is the safer behavior for hostile inputs.

### 3.2 Password handling

Revisions 5 and 6 no longer use the fixed 32-byte PDF password padding string.
The password bytes used for key retrieval are UTF-8 bytes truncated to 127
bytes. PDF 2.0 expects SASLprep/stringprep processing before UTF-8 conversion.
qpdf notes that viewers often do not implement this correctly; veraPDF accepts
UTF-8 bytes directly.

pdfv should preserve the current `PasswordSecret` public API and implement the
revision-specific conversion inside the decryption module:

- revisions 2-4: existing PDF padding/truncation to 32 bytes;
- revisions 5-6: UTF-8 secret bytes, truncated to 127 bytes for the
  Standard-security hash inputs;
- SASLprep is deferred unless a real fixture requires it, because the available
  Rust `stringprep` crate is old and would add supply-chain surface for an
  interoperability path that common readers do not reliably implement.

### 3.3 Algorithm 2.A key retrieval

Authentication should attempt the owner password first, then the user password,
matching the existing pdfv behavior and veraPDF/qpdf prior art.

For an owner-password attempt:

1. Compute `hash(password, owner_validation_salt, U, owner_context = true)`.
2. Compare it in constant time to `O[0..32]`.
3. If it matches, compute `hash(password, owner_key_salt, U, true)`.
4. Decrypt `/OE` with AES-256-CBC, zero IV, no padding. The 32-byte plaintext is
   the file encryption key.

For a user-password attempt:

1. Compute `hash(password, user_validation_salt, empty, owner_context = false)`.
2. Compare it in constant time to `U[0..32]`.
3. If it matches, compute `hash(password, user_key_salt, empty, false)`.
4. Decrypt `/UE` with AES-256-CBC, zero IV, no padding. The 32-byte plaintext is
   the file encryption key.

If neither password check matches, return encrypted status with the existing
safe warning `incorrect password`. Do not continue with an all-zero or empty
intermediate state.

### 3.4 Algorithm 2.B hashing

Revision 5 uses SHA-256 over:

```text
password || salt || optional U[0..48]
```

Revision 6 starts with the same SHA-256 value, then runs the hardened loop:

1. Build `K1 = password || K || optional U[0..48]`.
2. Repeat `K1` 64 times.
3. Encrypt the repeated bytes with AES-128-CBC, no padding, using `K[0..16]` as
   key and `K[16..32]` as IV.
4. Compute the sum of the first 16 encrypted bytes modulo 3.
5. Hash the encrypted bytes with SHA-256, SHA-384, or SHA-512 for modulo values
   0, 1, or 2 respectively.
6. After at least 64 rounds, stop when the last encrypted byte is less than or
   equal to `round - 32`.
7. Truncate the final hash to 32 bytes.

The loop uses bounded in-memory buffers. With the current password cap, the
largest repeated buffer is small: `(127 + 64 + 48) * 64 = 15,296` bytes.

### 3.5 AESV3 object decryption

For `/V >= 5`, the object-specific key derivation from revisions 2-4 is not
used. The recovered 32-byte file key is the data key for every encrypted string
and stream.

AESV3 string and stream bodies use AES-256-CBC with PKCS#7-compatible padding
validation and removal. Like AESV2, the first 16 bytes of each encrypted string
or stream are the IV, followed by ciphertext. Decrypted bytes remain hostile
input and must pass the existing
`max_decrypted_string_bytes` and `max_decrypted_stream_bytes` checks before
being fed back into object parsing or stream decoding.

Metadata handling stays consistent with Phase 9: if encryption metadata is
disabled, metadata streams are left encrypted/untouched rather than passed
through content decryption.

### 3.6 `/Perms` validation

Revisions 5 and 6 require `/Perms` validation in pdfv. Decrypt the 16-byte
`/Perms` value with AES-256-ECB using the file key. The cleartext format is:

```text
P little-endian 4 bytes || 0xff 0xff 0xff 0xff || T/F || "adb" || random 4 bytes
```

The stable validation prefix is the first 12 bytes. The trailing 4 bytes are
random and must not participate in equality checks. PDF 2.0 errata clarify that
the AES-256 operation is ECB mode, not CBC with a zero IV.

pdfv should treat an invalid `/Perms` prefix as encrypted/unsupported rather
than accepting decrypted content with only a warning. qpdf warns and proceeds,
but pdfv's validator reads hostile input and should prefer rejecting tampered
encryption metadata until a compatibility corpus demonstrates the need for a
looser mode.

### 3.7 Dependency plan

No new dependency is required for AES-256 block or CBC operations. Existing
`aes`, `cbc`, and `cipher` versions expose `Aes256`, CBC mode, and direct block
encrypt/decrypt traits.

Add `sha2 = "0.11.0"` when implementing revisions 5 and 6. It is current as of
2026-05-16, pure Rust, MIT OR Apache-2.0, and provides SHA-256, SHA-384, and
SHA-512.

Avoid adding `ecb` unless implementation simplicity clearly outweighs the
extra dependency. `/Perms` needs exactly one 16-byte AES-256 block, so direct
`Aes256::decrypt_block` through `cipher` is sufficient.

Avoid `stringprep` for the first AES-256 implementation. The latest crate is
0.1.5 with unknown Rust version metadata, and prior art already shows direct
UTF-8 bytes are the practical interoperability path for common readers.

### 3.8 Fixture availability

No AES-256 encrypted fixture was found in the vendored veraPDF parser,
validation, or app test resources. The local development environment also does
not have `qpdf` installed.

The implementation phase should therefore use deterministic generated fixtures,
matching the Phase 9 approach:

- R5 AESV3 with non-empty user password, opened with the user password;
- R5 AESV3 with distinct owner password, opened with the owner password;
- R6 AESV3 with non-empty user password, opened with the user password;
- R6 AESV3 with distinct owner password, opened with the owner password;
- wrong password returns `ValidationStatus::Encrypted`;
- malformed short `/O`, `/U`, `/OE`, `/UE`, and `/Perms` parse-fail;
- tampered `/Perms` returns encrypted/unsupported;
- AESV3 string and stream decryption are both covered under byte limits.

If qpdf is available in CI or developer machines, add an optional external
cross-check target later, but do not make normal tests depend on qpdf.

## 4. Gate Decision

AES-256 Standard security handler support is implementation-ready for a future
phase. The next implementation should extend the existing decryption module
rather than replace it:

- add `SecurityRevision::R5` and `SecurityRevision::R6`;
- add an AES-256 dictionary branch with validated R5/R6-only fields;
- add `CryptMethod::AesV3`;
- route authentication to revision-specific key retrieval;
- use the recovered file key directly for AESV3 object content;
- validate `/Perms` before decrypting document objects;
- add generated fixtures and unsupported/malformed coverage described above.

No public API changes are needed. The existing `PasswordSecret`,
`ValidationOptions`, CLI password-source policy, encrypted status, and redacted
reporting contracts remain valid.
