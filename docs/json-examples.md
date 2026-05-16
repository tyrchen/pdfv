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
        "id": "pdfv-m0",
        "name": "pdfv M0 built-in profile",
        "version": "0.1.0"
      },
      "isCompliant": true,
      "checksExecuted": 4,
      "rulesExecuted": 4,
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
