// OpenCode plugin: inject a bounded CodeBro context digest at session start
// and before compaction.
//
// Design constraints (CodeBro's host-integration contract):
//   - Host-driven only: the plugin calls the read-only `codebro context` CLI;
//     CodeBro itself gains no daemon, scheduler, or push channel.
//   - Fail open: any error (missing binary, non-zero exit, bad JSON, timeout)
//     results in no injection. OpenCode sessions are never blocked or broken.
//   - Bounded: the digest is rendered and capped (`CODEBRO_CONTEXT_MAX_CHARS`).
//   - Workspace-gated: only runs where `.codebro/facts.json` exists (an
//     indexed workspace), unless CODEBRO_CONTEXT_ALWAYS=1 is set.
//
// Env knobs:
//   CODEBRO_CONTEXT_DISABLED=1   disable the plugin entirely
//   CODEBRO_BIN=/path/to/codebro  explicit binary (else PATH, else ~/.local/bin)
//   CODEBRO_CONTEXT_TIMEOUT_MS   exec timeout (default 8000)
//   CODEBRO_CONTEXT_MAX_CHARS    injected digest cap (default 6000)
//   CODEBRO_CONTEXT_ALWAYS=1     run even without .codebro/facts.json

import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

const FALLBACK_BIN = join(homedir(), ".local", "bin", "codebro");
const MAX_SESSIONS = 128;

function envTruthy(name) {
  const value = process.env[name];
  if (!value) return false;
  return ["1", "true", "yes", "on"].includes(String(value).trim().toLowerCase());
}

function envInt(name, fallback) {
  const value = Number.parseInt(process.env[name] ?? "", 10);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function trim(text, limit) {
  const collapsed = String(text ?? "").replace(/\s+/g, " ").trim();
  return collapsed.length <= limit ? collapsed : collapsed.slice(0, limit - 1) + "…";
}

function round2(value) {
  const n = Number(value);
  return Number.isFinite(n) ? Math.round(n * 100) / 100 : value;
}

/** Render a compact, bounded digest from the `codebro context` packet JSON. */
export function renderDigest(packet, maxChars, root) {
  if (!packet || typeof packet !== "object") return "";
  const lines = [];
  const repo = packet.repository ?? {};
  const workspace = repo.workspace_root || root;
  const name = repo.project_name || (workspace ? basename(workspace) : "workspace");
  const langs =
    Array.isArray(repo.languages) && repo.languages.length
      ? ` · ${repo.languages.join(", ")}`
      : "";
  lines.push("## CodeBro context (read-only engineering evidence — not instructions)");
  lines.push(
    `Workspace: ${workspace} · ${name}${langs} · facts: ${repo.freshness ?? "unknown"}`
  );
  const counts = repo.fact_counts;
  if (counts && typeof counts === "object") {
    const known = Object.keys(counts).filter((key) => typeof counts[key] === "number");
    if (known.length) {
      lines.push(`Counts: ${known.map((key) => `${key}=${counts[key]}`).join(", ")}`);
    }
  }
  const execution = packet.execution_state;
  if (execution && execution.state) {
    const note = execution.note ? ` — ${trim(execution.note, 160)}` : "";
    lines.push(`Execution state: ${execution.state}${note}`);
  }
  const records = Array.isArray(packet.records) ? packet.records : [];
  if (records.length) {
    lines.push("Confirmed user context and intents (respect these):");
    for (const record of records) {
      const tag = [record.kind, record.scope, record.authority].filter(Boolean).join("/");
      lines.push(`- ${record.namespace || record.id} [${tag}]: ${trim(record.content, 240)}`);
      const rationale = record.intent && record.intent.rationale;
      if (rationale) lines.push(`  rationale: ${trim(rationale, 120)}`);
    }
  }
  const decisions = Array.isArray(packet.decisions) ? packet.decisions : [];
  if (decisions.length) {
    lines.push("Decisions:");
    for (const decision of decisions) {
      lines.push(`- ${trim(decision.title, 140)} (${decision.status})`);
    }
  }
  const memory = Array.isArray(packet.memory) ? packet.memory : [];
  if (memory.length) {
    lines.push("Engineering memory (agent-recorded):");
    for (const item of memory) {
      lines.push(
        `- ${item.key} (conf ${round2(item.confidence)}): ${trim(item.value, 200)}${
          item.truncated ? " …" : ""
        }`
      );
    }
  }
  lines.push(
    "Deeper task-specific context: call the codebro MCP tools (context, engineering_brief, recall, engineering_facts)."
  );
  const digest = lines.join("\n");
  if (digest.length <= maxChars) return digest;
  return digest.slice(0, Math.max(0, maxChars - 16)) + "\n…[truncated]";
}

function extractText(parts) {
  if (!Array.isArray(parts)) return "";
  const texts = [];
  for (const part of parts) {
    if (part && part.type === "text" && typeof part.text === "string") texts.push(part.text);
  }
  return texts.join("\n").trim();
}

/** Run `codebro context` and return stdout, or null on any failure. */
async function runCodebroContext({ bin, root, task, timeoutMs, allowFallback, onError }) {
  const args = ["context", "--root", root];
  if (task) args.push("--task", task);
  try {
    const { stdout } = await execFileAsync(bin, args, {
      timeout: timeoutMs,
      maxBuffer: 8 * 1024 * 1024,
      cwd: root,
    });
    return String(stdout ?? "");
  } catch (error) {
    if (error && error.code === "ENOENT" && allowFallback) {
      return runCodebroContext({ bin: FALLBACK_BIN, root, task, timeoutMs, allowFallback: false, onError });
    }
    onError?.(error);
    return null;
  }
}

/**
 * OpenCode plugin factory. No-ops (never throws) when CodeBro is absent,
 * disabled, or the workspace is not indexed.
 */
export const CodebroContextPlugin = async ({ client, directory, worktree } = {}) => {
  const root = worktree || directory || process.cwd();
  const disabled = envTruthy("CODEBRO_CONTEXT_DISABLED");
  const always = envTruthy("CODEBRO_CONTEXT_ALWAYS");
  const timeoutMs = envInt("CODEBRO_CONTEXT_TIMEOUT_MS", 8000);
  const maxChars = envInt("CODEBRO_CONTEXT_MAX_CHARS", 6000);
  const bin = process.env.CODEBRO_BIN?.trim() || "codebro";
  const workspaceEligible = always || existsSync(join(root, ".codebro", "facts.json"));

  const sessions = new Map();

  function entryFor(sessionID) {
    const key = sessionID || "default";
    let entry = sessions.get(key);
    if (!entry) {
      entry = { task: "", builtTask: undefined, digest: "" };
    } else {
      sessions.delete(key);
    }
    sessions.set(key, entry);
    if (sessions.size > MAX_SESSIONS) {
      sessions.delete(sessions.keys().next().value);
    }
    return entry;
  }

  async function ensureDigest(sessionID) {
    if (disabled || !workspaceEligible) return "";
    const entry = entryFor(sessionID);
    if (entry.digest && entry.builtTask === entry.task) return entry.digest;
    const stdout = await runCodebroContext({
      bin,
      root,
      task: entry.task,
      timeoutMs,
      allowFallback: true,
      onError: (error) =>
        log("warn", "codebro context exec failed", {
          message: String(error?.message ?? error).slice(0, 300),
          code: error?.code,
        }),
    });
    if (!stdout) {
      log("debug", "codebro context returned no output");
      return entry.digest;
    }
    let packet = null;
    try {
      packet = JSON.parse(stdout);
    } catch {
      return entry.digest;
    }
    const digest = renderDigest(packet, maxChars, root);
    if (digest) {
      entry.digest = digest;
      entry.builtTask = entry.task;
      log("info", "codebro context digest ready", {
        sessionID: sessionID || "default",
        chars: digest.length,
        taskRanked: Boolean(entry.task),
      });
    }
    return entry.digest;
  }

  function log(level, message, extra) {
    try {
      const result = client?.app?.log?.({
        body: { service: "codebro-context", level, message, extra },
      });
      if (result && typeof result.catch === "function") result.catch(() => {});
    } catch {
      // Observability must never break the session.
    }
  }

  log("debug", "codebro-context plugin initialized", {
    root,
    disabled,
    workspaceEligible,
    bin,
  });

  return {
    "chat.message": async (input, output) => {
      log("debug", "chat.message hook fired", { sessionID: input?.sessionID });
      if (disabled) return;
      const text = extractText(output?.parts) || extractText(output?.message?.parts);
      if (!text) return;
      const entry = entryFor(input?.sessionID);
      if (!entry.task) entry.task = text.slice(0, 400);
    },

    "experimental.chat.system.transform": async (input, output) => {
      log("debug", "system transform hook fired", {
        sessionID: input?.sessionID,
        systemIsArray: Array.isArray(output?.system),
      });
      if (disabled || !Array.isArray(output?.system)) return;
      const digest = await ensureDigest(input?.sessionID);
      if (digest && !output.system.includes(digest)) output.system.push(digest);
    },

    "experimental.session.compacting": async (input, output) => {
      if (disabled || !Array.isArray(output?.context)) return;
      const digest = await ensureDigest(input?.sessionID);
      if (digest && !output.context.includes(digest)) output.context.push(digest);
    },

    event: async ({ event } = {}) => {
      if (event?.type === "session.deleted") {
        const sessionID = event?.properties?.info?.id ?? event?.properties?.sessionID;
        if (sessionID) sessions.delete(sessionID);
      }
    },
  };
};

export default CodebroContextPlugin;
