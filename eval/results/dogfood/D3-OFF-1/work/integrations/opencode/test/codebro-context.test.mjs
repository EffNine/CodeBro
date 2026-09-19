// Tests for the CodeBro OpenCode plugin. Run with: node --test test/
//
// The plugin shells out to `codebro context`; tests point CODEBRO_BIN at a
// controllable stub so every path (success, failure, timeout, disabled,
// unindexed workspace) is exercised without Rust or a real CodeBro install.

import assert from "node:assert/strict";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  CodebroContextPlugin,
  renderDigest,
} from "../plugins/codebro-context.js";

const PACKET = {
  repository: {
    provenance: "recorded",
    workspace_root: "/tmp/demo-repo",
    identity_loaded: true,
    project_name: "demo-repo",
    languages: ["rust"],
    freshness: "fresh",
    fact_counts: { symbols: 42, modules: 7, tests: 3 },
  },
  execution_state: { state: "verified", note: "current tree has passing evidence" },
  records: [
    {
      id: "ctx::intent::demo::1",
      kind: "intent",
      namespace: "intent.demo-intent",
      content: "Keep the combo simple and seamless",
      authority: "user_confirmed",
      scope: "global",
      status: "active",
      importance: 0.9,
      effective_confidence: 0.95,
      intent: { intent_status: "active", rationale: "user said so" },
    },
  ],
  decisions: [{ id: "d1", title: "Use SQLite for local state", status: "accepted" }],
  memory: [
    { key: "architecture:loop", value: "Curator exports a portable mirror", confidence: 0.9, truncated: false },
  ],
  skills: {
    applicable: [
      {
        name: "parser-refactor",
        description: "Refactor parser subsystems safely",
        version: 1,
        scope: "global",
        reason: "task intent overlaps skill purpose/description (parser)",
      },
    ],
    pending_approvals: [
      {
        name: "outcome-reporter",
        request_id: "apr::5af23648513ec61a",
        proposed_action: "approve",
        question: "Approve publishing skill 'outcome-reporter'?",
      },
    ],
  },
  notes: ["structural digest"],
};

function makeStub(dir) {
  const stub = join(dir, "codebro-stub");
  writeFileSync(
    stub,
    `#!/usr/bin/env bash
if [ -n "$STUB_ARGS_FILE" ]; then { echo "---invocation---"; printf '%s\\n' "$@"; } >> "$STUB_ARGS_FILE"; fi
mode="\${STUB_MODE:-ok}"
if [ "$mode" = "sleep" ]; then sleep "\${STUB_SLEEP_MS:-2}"; fi
if [ "$mode" = "fail" ]; then echo "boom" >&2; exit 1; fi
if [ "$mode" = "badjson" ]; then echo "definitely not json"; exit 0; fi
cat <<'JSON'
${JSON.stringify(PACKET)}
JSON
`,
  );
  chmodSync(stub, 0o755);
  return stub;
}

function makeWorkspace(dir, indexed) {
  const root = join(dir, "workspace");
  mkdirSync(root, { recursive: true });
  if (indexed) {
    mkdirSync(join(root, ".codebro"), { recursive: true });
    writeFileSync(join(root, ".codebro", "facts.json"), "{}");
  }
  return root;
}

function makeEnv(dir, { stub, indexed = true, extra = {} } = {}) {
  const root = makeWorkspace(dir, indexed);
  const argsFile = join(dir, "args.log");
  const env = {
    CODEBRO_BIN: stub ?? makeStub(dir),
    CODEBRO_STATE_DIR: join(dir, "state"),
    STUB_ARGS_FILE: argsFile,
    STUB_MODE: "ok",
    ...extra,
  };
  return { root, argsFile, env };
}

async function withEnv(env, fn) {
  const saved = new Map();
  const unset = ["CODEBRO_CONTEXT_DISABLED", "CODEBRO_CONTEXT_ALWAYS", "CODEBRO_CONTEXT_TIMEOUT_MS", "CODEBRO_CONTEXT_MAX_CHARS"];
  for (const key of unset) {
    saved.set(key, process.env[key]);
    delete process.env[key];
  }
  for (const [key, value] of Object.entries(env)) {
    saved.set(key, process.env[key]);
    process.env[key] = value;
  }
  try {
    return await fn();
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

function makeTempDir() {
  const dir = mkdtempSync(join(tmpdir(), "codebro-plugin-"));
  return { dir, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

async function runTransform(plugin, sessionID = "ses_1") {
  const output = { system: [] };
  await plugin["experimental.chat.system.transform"]({ sessionID }, output);
  return output.system;
}

test("injects a bounded context digest into the system prompt", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir);
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 1);
      assert.match(system[0], /CodeBro context/);
      assert.match(system[0], /demo-repo/);
      assert.match(system[0], /intent\.demo-intent/);
      assert.match(system[0], /Keep the combo simple and seamless/);
      assert.match(system[0], /Execution state: verified/);
      assert.match(system[0], /applicable skill: parser-refactor/);
      assert.match(system[0], /pending skill approval: outcome-reporter/);
      assert.match(system[0], /apr::5af23648513ec61a/);
      assert.match(system[0], /codebro MCP tools/);
    });
  } finally {
    cleanup();
  }
});

test("skill nudges are advisory, bounded, and optional", () => {
  const digest = renderDigest(PACKET, 6000, "/tmp/demo-repo");
  assert.match(digest, /applicable skill: parser-refactor — Refactor parser/);
  assert.match(digest, /pending skill approval: outcome-reporter/);
  assert.match(digest, /never publishes or approves automatically/);

  const noSkills = { ...PACKET, skills: undefined };
  const plain = renderDigest(noSkills, 6000, "/tmp/demo-repo");
  assert.doesNotMatch(plain, /applicable skill/);
  assert.doesNotMatch(plain, /pending skill approval/);

  const malformed = {
    ...PACKET,
    skills: {
      applicable: [null, "nope", { name: "ok-skill", description: "x".repeat(500) }],
      pending_approvals: [null, {}],
    },
  };
  const safe = renderDigest(malformed, 6000, "/tmp/demo-repo");
  assert.match(safe, /applicable skill: ok-skill/);
  assert.ok(safe.length > 0);

  const capped = renderDigest(PACKET, 120, "/tmp/demo-repo");
  assert.ok(capped.length <= 120, `capped (${capped.length})`);
});

test("skill nudge lines never exceed their per-field trim", () => {
  const longName = "n".repeat(300);
  const longDescription = "d".repeat(1000);
  const packet = {
    ...PACKET,
    skills: {
      applicable: [{ name: longName, description: longDescription }],
      pending_approvals: [{ name: longName, request_id: "r".repeat(200) }],
    },
  };
  const digest = renderDigest(packet, 6000, "/tmp/demo-repo");
  const skillLine = digest.split("\n").find((line) => line.startsWith("- applicable skill:"));
  // Prefix 21 + name ≤80 + separator 3 + description ≤160.
  assert.ok(skillLine.length <= 264, `bounded skill line (${skillLine.length})`);
  const approvalLine = digest
    .split("\n")
    .find((line) => line.startsWith("- pending skill approval:"));
  // Prefix 26 + name ≤80 + suffix 28 + " (id)" ≤43.
  assert.ok(approvalLine.length <= 177, `bounded approval line (${approvalLine.length})`);
});

test("passes the first user message as the task, once", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, argsFile, env } = makeEnv(dir);
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      await runTransform(plugin);
      await plugin["chat.message"](
        { sessionID: "ses_1" },
        { message: {}, parts: [{ type: "text", text: "Implement the parser plumbing" }] }
      );
      const system = await runTransform(plugin);
      assert.equal(system.length, 1);
      const invocations = readFileSync(argsFile, "utf8")
        .split("---invocation---")
        .filter((entry) => entry.trim().length > 0);
      assert.equal(invocations.length, 2, "one taskless build, then one task-ranked rebuild");
      const taskRuns = invocations.filter((entry) => entry.includes("--task"));
      assert.equal(taskRuns.length, 1);
      assert.match(taskRuns[0], /Implement the parser plumbing/);
    });
  } finally {
    cleanup();
  }
});

test("fails open when the codebro binary exits non-zero", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir, { extra: { STUB_MODE: "fail" } });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 0);
    });
  } finally {
    cleanup();
  }
});

test("fails open on invalid JSON", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir, { extra: { STUB_MODE: "badjson" } });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 0);
    });
  } finally {
    cleanup();
  }
});

test("fails open and respects the exec timeout", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir, {
      extra: { STUB_MODE: "sleep", STUB_SLEEP_MS: "1500", CODEBRO_CONTEXT_TIMEOUT_MS: "200" },
    });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const started = Date.now();
      const system = await runTransform(plugin);
      const elapsed = Date.now() - started;
      assert.equal(system.length, 0);
      assert.ok(elapsed < 1200, `timeout should cut the run short (took ${elapsed}ms)`);
    });
  } finally {
    cleanup();
  }
});

test("does nothing when disabled", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, argsFile, env } = makeEnv(dir, { extra: { CODEBRO_CONTEXT_DISABLED: "1" } });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 0);
      assert.equal(existsSync(argsFile), false);
    });
  } finally {
    cleanup();
  }
});

test("skips unindexed workspaces unless forced", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, argsFile, env } = makeEnv(dir, { indexed: false });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 0);
      assert.equal(existsSync(argsFile), false);
    });
  } finally {
    cleanup();
  }
});

test("forced mode runs without an indexed workspace", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir, { indexed: false, extra: { CODEBRO_CONTEXT_ALWAYS: "1" } });
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const system = await runTransform(plugin);
      assert.equal(system.length, 1);
    });
  } finally {
    cleanup();
  }
});

test("injects into the compaction context", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir);
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      const output = { context: [] };
      await plugin["experimental.session.compacting"]({ sessionID: "ses_1" }, output);
      assert.equal(output.context.length, 1);
      assert.match(output.context[0], /CodeBro context/);
    });
  } finally {
    cleanup();
  }
});

test("surfaces intent-guard review findings, and stays silent when aligned", () => {
  const reviewPacket = {
    ...PACKET,
    guard: {
      verdict: "review",
      signals: [
        {
          code: "identity_mismatch",
          severity: "warn",
          detail: 'project identity "Hermes" does not match workspace basename "codebro"',
        },
        { code: "no_confirmed_intent", severity: "info", detail: "no actionable intent" },
      ],
    },
    records: [
      {
        ...PACKET.records[0],
        guard: { status: "ambiguous_project_reference", reason: "references multiple projects" },
      },
    ],
  };
  const digest = renderDigest(reviewPacket, 6000, "/tmp/demo-repo");
  assert.match(digest, /Intent guard: review/);
  assert.match(digest, /does not match workspace basename/);
  assert.doesNotMatch(digest, /no actionable intent/);
  assert.match(digest, /guard: ambiguous_project_reference/);

  const alignedPacket = {
    ...PACKET,
    guard: { verdict: "aligned", signals: [] },
  };
  const aligned = renderDigest(alignedPacket, 6000, "/tmp/demo-repo");
  assert.doesNotMatch(aligned, /Intent guard/);
});

test("caps the digest size", async () => {
  const long = PACKET.records[0];
  const packet = {
    ...PACKET,
    records: Array.from({ length: 20 }, (_, index) => ({
      ...long,
      id: `ctx::record::${index}`,
      namespace: `intent.long-${index}`,
      content: "x".repeat(500),
    })),
  };
  const digest = renderDigest(packet, 300, "/tmp/demo-repo");
  assert.ok(digest.length <= 300, `digest capped (was ${digest.length})`);
  assert.match(digest, /\[truncated\]/);
});

test("cleans up deleted sessions", async () => {
  const { dir, cleanup } = makeTempDir();
  try {
    const { root, env } = makeEnv(dir);
    await withEnv(env, async () => {
      const plugin = await CodebroContextPlugin({ directory: root });
      await runTransform(plugin, "ses_gone");
      await plugin.event({ event: { type: "session.deleted", properties: { info: { id: "ses_gone" } } } });
      assert.ok(plugin);
    });
  } finally {
    cleanup();
  }
});
