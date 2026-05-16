# 61-crates-and-features: Workspace, Dependencies, and Feature Flags

Status: draft · Owner: pdfv · Depends on: [00-prd.md](./00-prd.md)

## 1. Purpose

This spec keeps crate boundaries, dependency choices, and feature flags coherent. Dependency versions below were checked with `cargo search` on 2026-05-15, using Rust `1.95.0` as the current stable toolchain on this machine. The official Rust release listing identifies Rust `1.95.0` as the latest stable release on 2026-04-16.

## 2. Workspace shape

```text
crates/
  core/        public library API, parser, profiles, validation, reports
apps/
  cli/         pdfv binary
```

Phase 0 decided that `apps/server` is removed until a network service is explicitly scoped. M0 uses `apps/cli` as the only application crate.

## 3. Candidate dependencies

| Area | Crate | Current version checked | Use |
| --- | --- | --- | --- |
| CLI | `clap` | 4.6.1 | Derive-based CLI parsing. |
| Parser combinators | `winnow` | 1.0.3 | Bounded byte grammar parsers where useful. |
| Errors | `thiserror` | 2.0.18 | Library error enums. |
| CLI errors | `anyhow` | 1.0.102 in workspace | CLI context only. |
| Serialization | `serde` | 1.0.228 | Reports/config. |
| JSON | `serde_json` | 1.0.149 | JSON report output. |
| XML profiles | `quick-xml` | 0.40.1 | Custom profile/generator XML parsing. |
| Config | `config` | 0.15.23 | YAML runtime config. |
| Builders | `typed-builder` | 0.23.2 | Public config builders. |
| Tracing | `tracing` | 0.1.44 | Structured diagnostics. |
| Compression | `flate2` | 1.1.9 | Flate stream decode behind limits. |
| Parallel CLI | `rayon` | 1.12.0 | Bounded batch file validation. |
| Benchmarks | `criterion` | 0.8.2 | Performance gates after parser stabilizes. |
| Property tests | `proptest` | 1.11.0 | Parser/rule invariants. |
| Parameterized tests | `rstest` | 0.26.1 | Rule and fixture matrices. |
| Temp files | `tempfile` | 3.27.0 | CLI/config/report tests. |
| Secrets | `secrecy` | 0.10.3 checked 2026-05-16 | Redacted password storage for encrypted PDFs. |
| Constant-time comparison | `subtle` | 2.6.1 checked 2026-05-16 | Password authentication byte comparisons. |
| PDF legacy hash | `md-5` | 0.11.0 checked 2026-05-16 | Standard security handler revisions 2-4. |
| PDF legacy cipher | `rc4` | 0.2.0 checked 2026-05-16 | RC4 decryption required by older encrypted PDFs. |
| AES | `aes` | 0.9.0 checked 2026-05-16 | AESV2/AESV3 block cipher. |
| CBC mode | `cbc` | 0.2.0 checked 2026-05-16 | AES-CBC stream/string decryption. |
| Cipher traits | `cipher` | 0.5.1 checked 2026-05-16 | Shared crypto trait imports. |
| SHA-2 | `sha2` | 0.11.0 checked 2026-05-16 | Future Standard security handler revisions 5-6 spike. |

Avoid `serde_yaml` for new code because `cargo search` reports `0.9.34+deprecated`. Prefer `config` with a maintained YAML backend selected by that crate, or reassess with a dedicated dependency spike if direct YAML serialization is needed.

## 4. Feature flags

`pdfv-core`:

- `default = ["json", "flate"]`
- `json` enables JSON report serialization.
- `flate` enables FlateDecode support.
- `custom-profiles` enables XML profile loading.
- `decrypt` enables password-protected PDF decryption support and the crypto dependencies scoped by [14-password-decryption-design.md](./14-password-decryption-design.md).
- `bench` enables benchmark-only helpers.
- No feature may enable `unsafe` code.

`pdfv`:

- default includes `pdfv-core/json`, `pdfv-core/flate`, `config`, `tracing-subscriber`, `clap`.

## 5. Toolchain and lints

`rust-toolchain.toml` pins the latest stable Rust per AGENTS.md. Every crate root uses:

```rust
#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
```

CI and local verification run:

- `cargo build`
- `cargo test`
- `cargo +nightly fmt --all -- --check`
- `cargo clippy -- -D warnings -W clippy::pedantic`
- `cargo audit`
- `cargo deny check`

## 6. Cross-references

- ← Depends on: [00-prd.md](./00-prd.md)
- → Constrains: all implementation specs
- ↔ Related research: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md)
