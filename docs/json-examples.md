# JSON and Config Examples

Status: draft · Owner: pdfv · Last updated: 2026-05-16

## Single Validation

The JSON report uses camelCase fields and includes source summary, status, selected flavours, profile reports, parse facts, warnings, and task durations. The shape below is a shortened fixture-derived example; tests pin the full serializer shape.

```json
{
  "engineVersion": "0.1.0",
  "source": {
    "kind": "file",
    "path": "tests/fixtures/minimal-valid.pdf",
    "bytes": 80
  },
  "status": "valid",
  "flavours": [
    {
      "family": "pdfa",
      "part": 1,
      "conformance": "b"
    }
  ],
  "profileReports": [
    {
      "profile": {
        "id": "pdfv-m4",
        "name": "pdfv M4 built-in profile",
        "version": "0.1.0"
      },
      "isCompliant": true,
      "checksExecuted": 3,
      "rulesExecuted": 3,
      "failedRules": 0,
      "failedAssertions": [],
      "passedAssertions": [],
      "unsupportedRules": []
    }
  ],
  "parseFacts": [],
  "warnings": [],
  "taskDurations": []
}
```

## Batch Validation

Recursive validation emits a batch report even when only one PDF is discovered. This example is shortened from the recursive fixture test.

```json
{
  "items": [
    {
      "status": "valid",
      "source": {
        "kind": "file"
      }
    }
  ],
  "summary": {
    "totalFiles": 1,
    "valid": 1,
    "invalid": 0,
    "parseFailures": 0,
    "encrypted": 0,
    "incomplete": 0,
    "internalErrors": 0,
    "elapsedMillis": 0,
    "worstExitCategory": "success"
  },
  "warnings": []
}
```

## YAML Config

```yaml
validation:
  flavour: auto
  recordPassedAssertions: false
resources:
  maxFileBytes: 268435456
  maxObjects: 1000000
  maxObjectDepth: 128
  maxArrayLen: 65536
  maxDictEntries: 16384
  maxNameBytes: 127
  maxStringBytes: 1048576
  maxStreamDeclaredBytes: 134217728
  maxStreamDecodeBytes: 268435456
  maxParseFacts: 100000
output:
  format: json
  path: report.json
  redactPaths: true
```

## XML Compatibility

Use `--format xml` when a workflow expects a machine-readable XML report. `mrr` is accepted as a deprecated CLI alias for compatibility with older veraPDF scripts.

```xml
<?xml version="1.0" encoding="utf-8"?>
<report>
  <buildInformation>
    <releaseDetails id="pdfv-core" version="0.1.0"></releaseDetails>
  </buildInformation>
  <jobs>
    <job>
      <item size="80">
        <name>tests/fixtures/minimal-valid.pdf</name>
      </item>
      <validationReport profileName="pdfv M4 built-in profile" statement="PDF file is compliant with Validation Profile requirements." isCompliant="true">
        <details passedRules="3" failedRules="0" passedChecks="3" failedChecks="0" unsupportedRules="0"></details>
      </validationReport>
    </job>
  </jobs>
  <batchSummary totalJobs="1" failedToParse="0" encrypted="0" incomplete="0" internalErrors="0">
    <validationReports compliant="1" nonCompliant="0" failedJobs="0">1</validationReports>
    <duration elapsedMillis="0"></duration>
  </batchSummary>
</report>
```
