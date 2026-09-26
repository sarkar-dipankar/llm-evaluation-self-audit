#!/usr/bin/env bash
# Orchestrate the experiments over the corpus.
#
# Builds the promptdbg CLI from the sibling tool repo, then runs deterministic
# (provider-free) coverage + mutation measurement over the corpus and writes
# tidy results into results/. This is the experiment driver; the tool itself
# lives in ./tool and is invoked, never modified, from here.
#
# Usage:
#   scripts/run_experiments.sh [corpus_dir] [contexts.json]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOL_REPO="${TOOL_DIR:-$HERE/tool}"
CORPUS="${1:-$HERE/corpus/synthetic}"
CONTEXTS="${2:-$HERE/corpus/contexts/default.contexts.json}"
RESULTS="$HERE/results"
CACHE="$HERE/.cache"

mkdir -p "$RESULTS" "$CACHE"

echo "==> Building promptdbg (release) from $TOOL_REPO"
cargo build --release --manifest-path "$TOOL_REPO/Cargo.toml" -q
BIN="$TOOL_REPO/target/release/promptdbg"

CTXDIR="$HERE/corpus/contexts"
echo "==> Generating per-prompt coverage-directed contexts into $CTXDIR"
mkdir -p "$CTXDIR"
shopt -s nullglob
for f in "$CORPUS"/*.rtpl; do
    stem="$(basename "$f" .prompt.rtpl)"
    "$BIN" gen-contexts "$f" --out "$CTXDIR/$stem.contexts.json" 2>/dev/null || true
done

echo "==> Batch coverage + mutation over $CORPUS (per-prompt contexts)"
"$BIN" --cache-dir "$CACHE" bench "$CORPUS" \
    --contexts-dir "$CTXDIR" --contexts "$CONTEXTS" --mutation --format csv \
    > "$RESULTS/bench.csv"
echo "    wrote $RESULTS/bench.csv"

echo "==> Per-prompt coverage + mutation JSON"
mkdir -p "$RESULTS/coverage" "$RESULTS/mutation"
shopt -s nullglob
for f in "$CORPUS"/*.rtpl; do
    name="$(basename "$f" .prompt.rtpl)"
    "$BIN" --cache-dir "$CACHE" coverage "$f" --contexts "$CONTEXTS" --format json \
        > "$RESULTS/coverage/$name.json"
    "$BIN" --cache-dir "$CACHE" mutate "$f" --contexts "$CONTEXTS" --format json \
        > "$RESULTS/mutation/$name.json"
done
echo "    wrote per-prompt JSON to $RESULTS/{coverage,mutation}/"

echo "==> Summary"
python3 "$HERE/scripts/analyze_results.py" "$RESULTS/bench.csv" || true

echo "Done."
