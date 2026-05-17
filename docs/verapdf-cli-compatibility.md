# veraPDF CLI Compatibility

Status: current compatibility inventory. Phase 20 owns validation feature extraction and policy checks; Phase 21 owns raw/HTML reports and conservative metadata repair.

pdfv exposes migration-oriented veraPDF surfaces where the Rust implementation has a bounded backing model. Unsupported veraPDF options are not accepted as silent no-ops.

## Supported

- `pdfv validate --format json|json-pretty|text|xml|mrr|raw|html`
- `pdfv validate --extract [all|family,...]`
- `pdfv validate --policy-file <path>`
- `pdfv validate --flavour <auto|pdfa-*|pdfua-*|wtpdf-*>`
- `pdfv validate --default-flavour <pdfa-*|pdfua-*|wtpdf-*>`
- `pdfv validate --recursive --non-pdf-extension`
- `pdfv repair-metadata <paths>... --output-dir <dir> [--prefix <prefix>]`

`mrr` is accepted as a deprecated alias for `xml`. `raw` writes a processor-style XML report. `html` writes a static report with no external network assets.

For migration scripts, pdfv also accepts veraPDF-style aliases `--defaultflavour`, `--recurse`, and `--nonpdfext`.

## Intentional Differences

- pdfv does not provide a literal `--password <text>` argument. Use `--password-stdin`, `--password-file`, or `--password-env <ENV_VAR>` so password material is not exposed through shell history or process listings.
- Metadata repair is never mixed into `validate`. Use the separate `repair-metadata` command.
- Metadata repair never modifies inputs in place. Outputs are written atomically under `--output-dir`.
- Current metadata repair is conservative: valid single-flavour inputs are copied unchanged with a `noAction` report, and unsupported cases are refused with structured reasons.
- Policy files use pdfv's bounded YAML policy format rather than arbitrary Schematron or XSLT.

## Out of Scope

- GUI, server mode, installers, and auto-update workflows.
- Dynamic plugins, arbitrary Schematron/XSLT execution, or network-backed policy execution.
- Repairing encrypted PDFs.
- Repairing invalid or incomplete validation results until safe metadata rewrite support is implemented.
- ZIP input processing, progress display, embedded logs in reports, and validation-off policy-only execution.
