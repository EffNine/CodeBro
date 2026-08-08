# Dataset: Tools (Tool Calling)

**Category folder**: `datasets/tools/` · provider-neutral.

## Benchmark

- **Purpose**: prove the model can invoke tools conforming to a declared
  JSON-Schema (including `strict` mode).
- **Inputs**: user task + schema definitions (functions), model's tools param.
- **Expected Behaviour**: correct tool call name + args; no hallucinated tool;
  loop terminates when no tool needed.
- **Success**: tool_success ≥ threshold; schema-adherent args; correct arg values.
- **Failure**: malformed args / unknown tool / infinite tool loop.
- **Mandatory Metrics**: tool_success, structured_compliance.
- **Repeatability**: seed; repeats ≥3; budget.
- **Replay**: hashed tool-call results for zero-token compare.

## Datasets

| ID | Version | Purpose | Difficulty | Expected behaviour | Tags |
|----|---------|---------|----------|--------------------|------|
| tool-return-weather | 1.0.0 | call weather tool with args | medium | correct location + date string | [tools, weather] |
| tool-filter-search | 1.0.0 | choose among several tools | medium | select the right tool | [tools, routing] |
| tool-loops | 2.0.0 | multi-round reasoning↔tool | hard | terminates, no dup calls | [tools, multi_round] |

No provider-specific tool outputs; tool models not assumptions about a vendor SDK.
`strict`/schema is fair to test if provider documents support.