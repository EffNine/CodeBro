#!/usr/bin/env bash
# Indexing benchmark: small / medium / large synthetic repositories.
#
# Measures cold-init wall time, warm (parse-cache) time, and facts size.
# Deterministic repos are generated from a fixed seed pattern so runs are
# comparable across commits. Results print as aligned rows; use
# `scripts/bench.sh --save <label>` to append to docs/benchmark/results.md.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
binary="${CODEBRO_BIN:-$root/target/release/codebro}"
save_label=""
if [ "${1:-}" = "--save" ]; then
    save_label="${2:?--save requires a label}"
fi

[ -x "$binary" ] || { echo "build first: cargo build --release" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

gen_repo() {
    # gen_repo <dir> <crates> <modules_per_crate> <fns_per_module>
    local dir="$1" crates="$2" mods="$3" fns="$4"
    mkdir -p "$dir"
    cat > "$dir/Cargo.toml" <<EOF
[workspace]
members = ["crates/*"]
EOF
    for c in $(seq 1 "$crates"); do
        local crate="crate_$c"
        mkdir -p "$dir/crates/$crate/src"
        cat > "$dir/crates/$crate/Cargo.toml" <<EOF
[package]
name = "$crate"
version = "0.1.0"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }
EOF
        : > "$dir/crates/$crate/src/lib.rs"
        for m in $(seq 1 "$mods"); do
            {
                echo "// module $m of $crate — includes \`handler_$m\` and routes."
                echo "/// #[get(\"/api/v$m/items\")]"
                echo "pub fn handler_$m() -> u32 { $m }"
                for f in $(seq 1 "$fns"); do
                    echo "pub fn fn_${m}_$f(x: u32) -> u32 { x + ${m} + $f }"
                done
            } >> "$dir/crates/$crate/src/lib.rs"
        done
    done
}

bench() {
    local name="$1" dir="$2"
    rm -rf "$dir/.codebro"
    local t0 t1
    t0=$(date +%s%N)
    "$binary" init --root "$dir" >/dev/null
    t1=$(date +%s%N)
    local cold=$(( (t1 - t0) / 1000000 ))
    t0=$(date +%s%N)
    "$binary" init --root "$dir" >/dev/null
    t1=$(date +%s%N)
    local warm=$(( (t1 - t0) / 1000000 ))
    local size
    size=$(du -sk "$dir/.codebro/facts.json" | cut -f1)
    printf "%-8s %10s ms %10s ms %8s KiB\n" "$name" "$cold" "$warm" "$size"
    if [ -n "$save_label" ]; then
        printf "| %s | %s | %s | %s |\n" "$save_label" "$cold ms" "$warm ms" "${size} KiB" \
            >> "$root/docs/benchmark/results.md"
    fi
}

echo "repo     cold-init   warm-init   facts.json"
gen_repo "$work/small"  2 4 20
bench small "$work/small"
gen_repo "$work/medium" 8 16 40
bench medium "$work/medium"
gen_repo "$work/large" 24 48 80
bench large "$work/large"
