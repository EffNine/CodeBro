# CodeBro Documentation

Engineering governance, planning, and reporting documents for CodeBro.

## Start here

- **[design/MCP_SERVER.md](design/MCP_SERVER.md)** — the current product:
  the engineering context layer exposed as an MCP server (`codebro serve` /
  `codebro init` / `codebro doctor`), verified end-to-end with OpenCode
  (A/B comparison + auto-detection tests)

## Structure

```
docs/
├── design/                # Current + planned architecture
│   └── MCP_SERVER.md          # MCP server design + verified tests
├── SOP/                  # Standard Operating Procedures
│   ├── codebro_sop_v1.md     # Master SOP
│   ├── development_protocol.md
│   ├── validation_protocol.md
│   ├── benchmark_protocol.md
│   ├── release_protocol.md
│   └── regression_protocol.md
├── RFC/                  # Request for Comments
│   ├── template.md
│   └── (accepted RFCs)
├── ADR/                  # Architecture Decision Records
│   ├── template.md
│   └── (accepted ADRs)
├── reports/              # Phase and regression reports
│   ├── phase_report_template.md
│   └── regressions/
└── roadmap/              # Development planning
    ├── roadmap.md
    ├── milestones.md
    └── feature_matrix.md
```

## Quick Start

1. Read `design/MCP_SERVER.md` for the current engineering-runtime design
2. Read `SOP/codebro_sop_v1.md` for the overall governance framework
3. Read `roadmap/roadmap.md` for the development plan
4. Use `RFC/template.md` to propose new features
5. Use `ADR/template.md` to document architectural decisions
6. Use `reports/phase_report_template.md` to report phase results
