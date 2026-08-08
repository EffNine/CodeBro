# Dataset: Reasoning

**Category folder**: `datasets/reasoning/` · provider-neutral.

## Benchmark

- **Purpose**: prove step-by-step reasoning correctness (thinking mode) on
  logic/math/multi-hop.
- **Inputs**: word problem + thinking-mode flags (e.g. effort) + context.
- **Expected Behaviour**: correct final answer; CoT internally consistent.
- **Success**: accuracy ≥ threshold; CoT consistency; determinism across repeats.
- **Failure**: plausible-but-wrong answer; contradiction between reasoning of the steps and answer.
- **Mandatory Metrics**: accuracy, determinism, reasoning consistency (derived).
- **Repeatability**: fixed seed; repeats ≥3.
- **Replay**: hashed verdict + reasoning snapshots.

## Datasets

| ID | Version | Purpose | Difficulty | Expected behaviour | Tags |
|----|---------|---------|------------|--------------------|------|
| logic-deduction | 1.0.0 | Boolean & arithmetic logic chains | medium | provable answer, minimal unsound steps | [reasoning, logic] |
| ambiguity-mbop | 1.0.0 | arithmetic order-of-operations | medium | correct precedence | [reasoning, arithmetic] |
| multi-hop-qa | 1.0.0 | 2-hop question answering | hard | answer traceable to two premises | [reasoning, qa] |

All IDs are provider-neutral; set via `reasoning` files (no vendor prompt
allowed). Working rule: reasoning datasets never reference any provider.