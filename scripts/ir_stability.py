#!/usr/bin/env python3
"""RQ4: measure the stability of LLM-inferred prompt IR.

For each prompt × model, runs `promptdbg analyze --engine llm --no-lint` N times and
quantifies how much the inferred structure varies across runs:

  - mean_nodes / std_nodes   : node-count variation
  - mean_jaccard             : mean pairwise Jaccard of node-ID sets across runs
                               (1.0 = identical structure every time)
  - id_stability             : |IDs present in ALL runs| / |IDs in ANY run|
  - mean_confidence          : mean of the IR's self-reported confidence

This is the empirical motivation for the deterministic annotation engine: it shows
how non-reproducible inferred prompt structure is on open models.

Caching is intentionally NOT used (no --cache-dir), so each run is an independent
inference. Usage:

  OLLAMA_API_KEY=... python scripts/ir_stability.py \
      --models gemma3:12b,gpt-oss:20b --repeats 5
"""
from __future__ import annotations

import argparse
import csv
import itertools
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"


def jaccard(a: set, b: set) -> float:
    if not a and not b:
        return 1.0
    return len(a & b) / len(a | b)


def analyze(binary: str, prompt: Path, model: str, timeout: int) -> dict | None:
    """Run one LLM IR inference; return parsed IR dict or None on failure."""
    env = dict(os.environ, OLLAMA_MODEL=model)
    try:
        out = subprocess.run(
            [binary, "analyze", str(prompt), "--engine", "llm",
             "--no-lint", "--format", "json"],
            capture_output=True, text=True, env=env, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return None
    if out.returncode != 0 or not out.stdout.strip():
        return None
    try:
        return json.loads(out.stdout)["ir"]
    except (json.JSONDecodeError, KeyError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--glob", default="*.rtpl",
                    help="filename glob (e.g. '*.md' for real GitHub skills)")
    ap.add_argument("--models", default="gemma3:12b")
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--out", default=str(HERE / "results" / "ir_stability.csv"))
    ap.add_argument("--max", type=int, default=0, help="cap prompts (0=no cap)")
    ap.add_argument("--save-irs", default="", help="dir to save raw IR JSONs per run (enables drift decomposition later)")
    args = ap.parse_args()

    if not os.environ.get("OLLAMA_API_KEY") and "OPENAI_API_KEY" not in os.environ:
        sys.exit("Set OLLAMA_API_KEY (or another provider key) first.")
    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    models = [m.strip() for m in args.models.split(",") if m.strip()]
    prompts = sorted(Path(args.corpus).glob(args.glob))
    if args.max > 0:
        prompts = prompts[:args.max]
    if not prompts:
        sys.exit(f"No .rtpl prompts under {args.corpus}")

    rows = []
    for prompt, model in itertools.product(prompts, models):
        irs = []
        for r in range(args.repeats):
            ir = analyze(binary, prompt, model, args.timeout)
            if ir is not None:
                irs.append(ir)
                # Optionally persist raw IR for drift decomposition / later analysis.
                if args.save_irs:
                    d = Path(args.save_irs) / model.replace(":", "_")
                    d.mkdir(parents=True, exist_ok=True)
                    (d / f"{prompt.stem}_r{r}.json").write_text(json.dumps(ir))
            print(f"  {prompt.name} [{model}] run {r + 1}/{args.repeats}: "
                  f"{'ok' if ir is not None else 'FAILED'}", file=sys.stderr)
        if len(irs) < 2:
            print(f"  {prompt.name} [{model}]: <2 successful runs, skipping",
                  file=sys.stderr)
            continue

        id_sets = [{n["id"] for n in ir["nodes"]} for ir in irs]
        counts = [len(ir["nodes"]) for ir in irs]
        confs = [ir.get("meta", {}).get("confidence", float("nan")) for ir in irs]
        pair_jac = [jaccard(a, b) for a, b in itertools.combinations(id_sets, 2)]
        all_ids = set().union(*id_sets)
        common_ids = set.intersection(*id_sets)

        rows.append({
            "prompt": prompt.name,
            "model": model,
            "successful_runs": len(irs),
            "mean_nodes": round(statistics.mean(counts), 2),
            "std_nodes": round(statistics.pstdev(counts), 2),
            "mean_jaccard": round(statistics.mean(pair_jac), 3),
            "id_stability": round(len(common_ids) / len(all_ids), 3) if all_ids else 1.0,
            "mean_confidence": round(statistics.mean(confs), 3),
        })
        print(f"== {prompt.name} [{model}]: jaccard={rows[-1]['mean_jaccard']} "
              f"id_stability={rows[-1]['id_stability']}", file=sys.stderr)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    fields = ["prompt", "model", "successful_runs", "mean_nodes", "std_nodes",
              "mean_jaccard", "id_stability", "mean_confidence"]
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    print(f"\nWrote {len(rows)} rows to {out}")
    if rows:
        mj = statistics.mean(r["mean_jaccard"] for r in rows)
        mi = statistics.mean(r["id_stability"] for r in rows)
        print(f"Corpus mean: jaccard={mj:.3f}  id_stability={mi:.3f}")
        print("(1.0 = perfectly reproducible structure; lower = inference drift)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
