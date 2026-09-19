# Dataset: Structured Output

**Category folder**: `datasets/structured_output/` · provider-neutral.

## Benchmark

- **Purpose**: prove schema-enforced structured outputs (not just "JSON-shaped");
  the model must honor a fixed JSON Schema field-by-field.
- **Inputs**: prompt describing data + target JSON Schema (tool-strict, if
  provider documents it).
- **Expected Behaviour**: output parses; every field matches schema; no extra fields.
- **Success**: structured_valid == 1 (or ≥ threshold); type accuracy per field.
- **Failure**: missing/extra field, type mismatch, inval json.
- **Metrics**: structured_valid, determinism, json_parse rate.
- **Replay**: hashed schemas + parsed shapes.

## Datasets

| ID | Version | Purpose | Difficulty | Expected behaviour | Tags |
|----|---------|---------|----------|--------------------|------|
| structured-profile | 1.0.0 | user→profile object | medium | exact field match to schema | [structured, schema] |
| structured-report | 2.0.0 | nested report object | hard | nested + required props | [structured, schema, nested] |
| structured-array | 1.0.0 | list of objects | hard | array with items validated | [structured, array] |

Schema files are stored as `.schema.json` beside each entry; prompts in
`.prompt`; goldens in `.golden.json`. No provider is named.