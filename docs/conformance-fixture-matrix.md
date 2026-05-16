# Conformance Fixture Matrix

Status: draft · Owner: pdfv · Last updated: 2026-05-16

The current matrix is intentionally generated in-repo so normal tests have clear provenance and can run without external corpora.

| Fixture | Coverage | Expected status |
| --- | --- | --- |
| `tests/fixtures/minimal-valid.pdf` | Header, catalog, trailer, default built-in profile rules. | `valid` |
| `tests/fixtures/leading-bytes-invalid.pdf` | Recoverable header-offset violation. | `invalid` |
| `tests/fixtures/not-a-pdf.pdf` | Hostile non-PDF bytes and parse-failure reporting. | `parseFailed` |
| `tests/fixtures/xref-stream-object-stream-valid.pdf` | Xref stream parsing and object stream expansion. | `valid` |

## Profile Fixtures

Custom profile XML fixtures are generated inside CLI tests. They cover bounded XML loading, veraPDF-style rule IDs, object mapping, and expression import for simple boolean comparisons.

The built-in veraPDF import path is exercised against `vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1B.xml`. Supported rules become executable IR; unsupported JavaScript-style expressions are retained and reported as `unsupportedRules`.

M4 model-fact coverage is exercised by generated unit fixtures with page, font,
annotation, output-intent, and page-content-stream objects. Those fixtures keep
advanced object coverage local to the model graph tests until a licensed public
PDF/A corpus is added.

## Encrypted Fixtures

Password/decryption coverage is generated in unit and CLI tests rather than
checked in as binary PDFs. Phase 9 covers RC4 revision 2, RC4 revision 3, RC4
revision 4 through crypt filters, AESV2 revision 4, missing/wrong password,
unsupported revision 6, unsupported public-key handlers, malformed `/Encrypt`,
metadata-unencrypted handling, and redaction.

The Phase 10 AES-256 spike did not find reusable AESV3 fixtures in the vendored
veraPDF trees. Future revision 5-6 implementation will add deterministic
generated R5/R6 AESV3 fixtures covering user password, owner password, wrong
password, tampered `/Perms`, malformed short key fields, and string/stream
decryption under resource limits.

## Opt-In veraPDF Corpus Tests

`apps/cli/tests/verapdf_corpus.rs` translates the upstream
`vendors/veraPDF-apps/tests/exit-status.sh` scenarios into `pdfv` CLI checks.
The tests are ignored by default because they require an external
`veraPDF-corpus` checkout and intentionally exercise broader conformance than
the local generated fixtures.

Run them with:

```bash
PDFV_VERAPDF_CORPUS_DIR=/path/to/veraPDF-corpus make test-conformance-verapdf
```

The current rows cover single pass, single fail, all-pass batch, all-fail
batch, mixed recursive directory validation, bad parameters, and parse failure.
Because the imported PDF/A-1b profile still contains unsupported rules, the
validation rows currently expect `incomplete` rather than veraPDF's final
valid/invalid decisions. They still guard parser/report behavior and known-pass
fixtures with zero failed rules until full model parity is implemented.
External corpus PDFs remain outside `tests/fixtures/`; any checked-in copy must
first be added to `tests/fixtures/manifest.md` with source, license, and
expected status metadata.
