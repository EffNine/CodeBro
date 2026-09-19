# Dataset: Long Context (Context Handling + Long Context)

**Category folder**: `datasets/long_context/` · provider-neutral.

## Benchmark

- **Purpose**: prove instruction-following, recall, and coherence across a long
  context window, and graceful behavior at scale.
- **Inputs**: long documents + prompts requiring: recall (needle), re-asserted
  instructions, and ordering.
- **Expected Behaviour**: instructions at the head of the context still honored;
  needle found; no lost/contradicted info.
- **Success**: accuracy ≥ threshold; recall exactness; degradation curve recorded
  (small → large context, e.g. 4K/32K/128K/512K).
- **Failure**: instruction drop after head, hallucinated insertion, 400-over-limit
  in-window at allowed sizes.
- **Mandatory Metrics**: accuracy, reliability, cost_per_task (input-heavy).
- **Replay**: golden needle answers hashed; degradation curve re-verifiable
  offline vs recorded.

## Datasets

| ID | Version | Purpose | Difficulty | Expected | Tags |
|----|---------|---------|-----------|----------|------|
| ctx-instruction-head | 1.0.0 | honor rule placed at top of context | medium | rule obeyed at end of long doc | [context, instruction] |
| ctx-needle | 1.0.0 | find token buried in long doc | hard | exact quote recall | [context, recall] |
| ctx-multi-doc | 1.0.0 | aggregate across several docs | hard | consistent cross-doc answer | [context, multi_doc] |
| ctx-scale-degrade | 1.0.0 | measure degrade curve vs length | medium | monotonic documented curve | [long_context, scale] |

Note: exact window lengths (1M etc.) are provider facts (see provider cards);
datasets stay generic and record `length` per case.