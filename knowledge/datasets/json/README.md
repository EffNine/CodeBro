# Dataset: JSON (JSON Output)

**Category folder**: `datasets/json/` · provider-neutral.

## Benchmark

- **Purpose**: prove the model emits *literally valid JSON* under
  `response_format` JSON-mode / strict.
- **Inputs**: prompt that MUST contain "json" keyword + example schema (per most
  providers' contract), plus `response_format` = json_object.
- **Expected Behaviour**: parseable JSON matching the schema in content.
- **Success**: validly parsed, required keys present, no extraneous prose.
- **Failure**: non-JSON content, accidentally empty body, truncated.
- **Mandatory**: structured_valid (json parse), empty_output_rate, determinism.
- **Replay**: hashed raw outputs re-validated offline.

## Datasets

| ID | Version | Purpose | Difficulty | Expected | Tags |
|----|---------|---------|----------|----------|------|
| json-simple | 1.0.0 | simple key/value object | easy | valid json only | [json] |
| json-nested | 1.0.0 | nested object + arrays | medium | schema-conforming | [json, nested] |
| json-empty-guard | 1.0.0 | stress empty-content path | hard | never empty body | [json, reliability] |

Added because official JSON-mode docs warn occasional empty-content; the
"empty-guard" dataset monitors that availability / drift.