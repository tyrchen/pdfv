# pdfv Test Fixture Manifest

| Path | Source | License | Expected parse status | Expected validation status | Reason |
| --- | --- | --- | --- | --- | --- |
| `minimal-valid.pdf` | Generated inline by the pdfv project from the M0 classic-xref fixture shape. | MPL-2.0 | success | valid | Minimal catalog-only PDF accepted by the M0 parser and built-in profile. |
| `leading-bytes-invalid.pdf` | Generated inline by the pdfv project by prefixing `minimal-valid.pdf` with bounded leading bytes. | MPL-2.0 | success | invalid | Exercises the M0 header-offset rule while keeping parsing recoverable. |
| `not-a-pdf.pdf` | Generated inline by the pdfv project as hostile non-PDF bytes. | MPL-2.0 | parseFailed | parseFailed | Exercises parse-failure reporting and CLI exit code 2. |

The fixture generator is intentionally simple and reproducible:

```text
minimal-valid.pdf = literal bytes in this directory
leading-bytes-invalid.pdf = "junk\n" + minimal-valid.pdf
not-a-pdf.pdf = "not a pdf\n"
```
