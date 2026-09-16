#!/usr/bin/env bash
# Generic single-workdir trial runner (harness v2: --format json session logs).
# Usage: run-trial.sh <trial-id> <task: t1|t3|t4|t5|t6> <ON|OFF> [second-session-variant]
#   The optional 4th arg (e.g. t3b-on) runs a second session in the same workdir
#   (T3 A->B handoff). Without it, a single session runs.
# Layout: /tmp/opencode/<phase>/<trial-id>/{work,prompt*.md,session*.log,env.json,meta.txt,
#   on.jsonc (ON only),work-grade/,grade*.log,workdir.diff,verdict.json}
# Environment under which trials were validated:
#   OpenCode 1.18.31, agnes/agnes-3.0-flash, CodeBro 1.1.0, rustc/cargo 1.97.1.
set -u
TRIAL="$1"; TASK="$2"; COND="$3"; SECOND="${4:-}"
BASE_DIR="${PHASE1B_BASE:-/tmp/opencode/phase1b}"
REPO=/home/afnan/projects/active/codebro
HARNESS=$REPO/eval/harness
OC=/home/afnan/.opencode/bin/opencode
MODEL=agnes/agnes-3.0-flash
SESS_TIMEOUT=570
R=$BASE_DIR/$TRIAL
FREEZE_COMMIT="${FREEZE_COMMIT:-c84048cc2d}"
STATE_SHA="${STATE_SHA:-8e34dc5560ca1914e78262bee46796fc5cbf2fb9c0d0b27e1102e1e8d9a10dfe}"

mkdir -p "$R/work"

if [ "$COND" = "ON" ]; then
  mkdir -p "$R/state"
  cp "$BASE_DIR/state-frozen.db" "$R/state/state.db"
  sed "s|<run-state-dir>|$R/state|" "$HARNESS/on-template.jsonc" > "$R/on.jsonc"
  export OPENCODE_CONFIG="$R/on.jsonc"
else
  export OPENCODE_CONFIG="$REPO/eval/overlays/opencode-off.jsonc"
fi

cat > "$R/env.json" <<EOF
{"opencode": "1.18.31", "model": "$MODEL", "codebro": "1.1.0",
 "format": "json",
 "rustc": "1.97.1", "cargo": "1.97.1", "os": "Linux 7.0.0-31-generic x86_64",
 "freeze_commit": "$FREEZE_COMMIT", "state_snapshot_sha256": "$STATE_SHA",
 "trial": "$TRIAL", "task": "$TASK", "condition": "$COND"}
EOF

run_session() { # $1=prompt-variant $2=log-tag
  sed "s|<workdir>|$R/work|g; s|<crate>|$TASK|g" "$HARNESS/prompts/$1.txt" > "$R/prompt-$2.md"
  ( cd "$R/work" && timeout $SESS_TIMEOUT $OC run --dir "$R/work" --auto --format json -m $MODEL "$(cat "$R/prompt-$2.md")" > "$R/session-$2.log" 2>&1 )
  echo "session-$2 exit: $?" | tee -a "$R/meta.txt"
}

echo "trial=$TRIAL task=$TASK cond=$COND started=$(date -u +%FT%TZ)" | tee "$R/meta.txt"
$REPO/eval/tasks/$TASK/setup.sh "$R/work"
ONOFF=$([ "$COND" = "ON" ] && echo on || echo off)
FIRST_VARIANT="$TASK-$ONOFF"
[ "$TASK" = "t3" ] && FIRST_VARIANT="t3a-$ONOFF"
run_session "$FIRST_VARIANT" a
if [ -n "$SECOND" ]; then
  run_session "$SECOND" b
fi
rm -rf "$R/work-grade"; cp -r "$R/work" "$R/work-grade"
( cd $REPO && ./eval/tasks/$TASK/grade.sh "$R/work-grade" > "$R/grade.log" 2>&1 )
echo "grade exit: $?" | tee -a "$R/meta.txt"
echo "finished=$(date -u +%FT%TZ)" | tee -a "$R/meta.txt"
