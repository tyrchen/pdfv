# M9 Live Conformance Review

The release comparison now checks the selected profiles as well as the outcome.
It requires complete veraPDF JSON and the corresponding exit code, and bounds
both output streams while the process runs. Release mode verifies fresh public T2
evidence before the private T3 run. The private gate requires 1,000 distinct
in-scope documents, 95% outcome agreement, and zero false-compliant rows.

The PDF/A binary-comment rule now uses the original bytes after the header, and
the classic xref rules check the actual line ending and subsection separator.
On `veraPDFtest-pass-a.pdf`, the PDF/A-1b rule failures for the binary comment
(`6.1.2-2`) and the xref separator (`6.1.4-2`) are gone.

## Live Evidence

The checked-in three-row T2 manifest still produces three classified
`Incomplete` differences with veraPDF Greenfield 1.30.4. An additional run used
eight fixtures at revision `bb75f4f0073d9350dfd058c0162a367e6fadf25e` of the
[veraPDF conformance corpus](https://github.com/veraPDF/veraPDF-corpus/tree/bb75f4f0073d9350dfd058c0162a367e6fadf25e).
The manifest records the exact source paths.

| Profile | veraPDF valid | veraPDF invalid | pdfv incomplete |
| --- | ---: | ---: | ---: |
| PDF/A-1b | 1 | 1 | 2 |
| PDF/A-4 | 1 | 1 | 2 |
| PDF/UA-1 | 1 | 1 | 2 |
| PDF/UA-2 | 1 | 1 | 2 |

The initial run found runtime unsupported-rule observations on all eight
supplemental rows and zero comparable false-compliant rows. The oracle exits
with a failure and writes the artifacts for review.

Run the supplemental manifest against a checkout of the pinned corpus.

```bash
PDFV_VERAPDF_BIN=/path/to/verapdf \
  PDFV_ORACLE_CORPUS_ROOT=/path/to/veraPDF-corpus \
  PDFV_ORACLE_CORPUS_MANIFEST=tests/oracle-t2-upstream-conformance.yml \
  PDFV_ORACLE_OUTPUT_DIR=target/parity/t2-upstream \
  make oracle-corpus
```

## Stream Dispatch Follow-up

Metadata and content wrappers can refer to a stream already visited by the
physical COS stream model. The validator now runs stream syntax rules on that
physical model once. This removed the stream property errors on both PDF/A-4
samples. The negative sample now agrees with veraPDF at `Invalid`. The
positive sample returns `Invalid` with 28 failed checks, so its remaining
assertions need rule-level comparison. The two PDF/A-1b rows each dropped from
30 unsupported evaluations to 26.

## Physical COS Primitive Follow-up

Integer, real, string, and name checks now execute on their respective parsed
types. The walk includes dictionary keys and compressed indirect objects.
Array positions and dictionary entries receive distinct report locations.
String length uses decoded bytes. Hexadecimal syntax rules use the original
digit count, including an odd final digit before padding. Serialized parsed
strings with syntax evidence use a `bytes` and `syntax` object. Legacy byte
arrays remain readable.

The checked-in T2 set reports one matching invalid decision and two incomplete
decisions. Unsupported evaluations on the two PDF/A-1b positive files fell
from 37 and 42 to 1 and 2. The remaining errors require font-program width and
CIDSet evidence. On the eight supplemental rows, the four PDF/A cases now
complete. Both negative cases agree with veraPDF, and both positive cases give
incorrect `Invalid` decisions. The four PDF/UA cases remain `Incomplete`.

## Release Blockers

- The automatic fallback still selects the ten-rule `pdfv-m4` profile. veraPDF
  selects the full fallback profile. The live comparison marks the two
  executions as incompatible.
- PDF/A and PDF/UA fixtures still exercise runtime gaps. For example, several
  COS primitive rule targets are mapped to the generic `object` family.
- The current workspace has zero private T3 release documents. Qualification
  requires the corpus and acceptance evidence specified in the
  [oracle verification plan](../../specs/27-oracle-corpus-verification-plan.md).
