#!/usr/bin/env python3
"""Ablation studies for the experiments.

Two ablations, isolating the contribution of distinct components:

  A. Oracle ablation (deterministic, provider-free)
     Mutation score under the structural TraceOracle vs the output-only OutputOracle.
     Quantifies how much the structural/coverage signal adds to fault detection
     beyond comparing rendered text.

  B. IR-context ablation for the linter (requires a provider)
     Lint counts WITH the inferred/annotation IR vs WITHOUT it (`--ablate-ir`).
     Quantifies how much the prompt IR contributes to the LLM linter's findings —
     i.e. whether "prompts as programs" helps downstream analysis.

Usage:
  # A only (no API key needed):
  python scripts/ablation.py --bin tool/target/release/promptdbg

  # A + B (B needs a provider):
  OLLAMA_API_KEY=... python scripts/ablation.py --ir-context --model gemma3:12b
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"


def run_json(binary: str, args: list[str], env: dict | None, timeout: int):
    out = subprocess.run(
        [binary, *args], capture_output=True, text=True, env=env, timeout=timeout
    )
    if out.returncode != 0 or not out.stdout.strip():
        return None
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return None


def oracle_ablation(binary: str, prompts, contexts: Path) -> list[dict]:
    rows = []
    for p in prompts:
        common = [str(p), "--contexts", str(contexts), "--engine", "annotation",
                  "--format", "json"]
        trace = run_json(binary, ["mutate", *common, "--oracle", "trace"], None, 120)
        output = run_json(binary, ["mutate", *common, "--oracle", "output"], None, 120)
        if not trace or not output:
            print(f"  {p.name}: mutate failed, skipping", file=sys.stderr)
            continue
        rows.append({
            "prompt": p.name,
            "total_mutants": trace["total_mutants"],
            "score_trace": round(trace["mutation_score"], 3),
            "score_output": round(output["mutation_score"], 3),
            "delta_trace_minus_output": round(
                trace["mutation_score"] - output["mutation_score"], 3),
            "killed_trace": trace["killed"],
            "killed_output": output["killed"],
        })
        print(f"  {p.name}: trace={rows[-1]['score_trace']} "
              f"output={rows[-1]['score_output']} "
              f"delta={rows[-1]['delta_trace_minus_output']}", file=sys.stderr)
    return rows


def ir_context_ablation(binary: str, prompts, model: str, timeout: int) -> list[dict]:
    env = dict(os.environ, OLLAMA_MODEL=model)
    rows = []
    for p in prompts:
        base = [str(p), "--engine", "annotation", "--format", "json"]
        with_ir = run_json(binary, ["analyze", *base], env, timeout)
        without_ir = run_json(binary, ["analyze", *base, "--ablate-ir"], env, timeout)
        if with_ir is None or without_ir is None:
            print(f"  {p.name}: analyze failed, skipping", file=sys.stderr)
            continue
        codes_with = {d.get("code") for d in with_ir["diagnostics"]}
        codes_without = {d.get("code") for d in without_ir["diagnostics"]}
        rows.append({
            "prompt": p.name,
            "diagnostics_with_ir": len(with_ir["diagnostics"]),
            "diagnostics_without_ir": len(without_ir["diagnostics"]),
            "codes_only_with_ir": len(codes_with - codes_without),
            "codes_only_without_ir": len(codes_without - codes_with),
        })
        print(f"  {p.name}: with_ir={rows[-1]['diagnostics_with_ir']} "
              f"without_ir={rows[-1]['diagnostics_without_ir']} "
              f"unique_to_ir={rows[-1]['codes_only_with_ir']}", file=sys.stderr)
    return rows


def write_csv(rows: list[dict], path: Path):
    if not rows:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"Wrote {len(rows)} rows to {path}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--contexts", default=str(HERE / "corpus" / "contexts" / "default.contexts.json"))
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--ir-context", action="store_true", help="also run ablation B (needs provider)")
    ap.add_argument("--model", default="gemma3:12b")
    ap.add_argument("--timeout", type=int, default=120)
    args = ap.parse_args()

    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    prompts = sorted(Path(args.corpus).glob("*.rtpl"))
    if not prompts:
        sys.exit(f"No .rtpl prompts under {args.corpus}")

    print("== Ablation A: mutation oracle (trace vs output) ==", file=sys.stderr)
    rows_a = oracle_ablation(binary, prompts, Path(args.contexts))
    write_csv(rows_a, HERE / "results" / "ablation_oracle.csv")
    if rows_a:
        md = sum(r["delta_trace_minus_output"] for r in rows_a) / len(rows_a)
        print(f"Mean mutation-score gain from structural oracle: {md:+.3f}\n")

    if args.ir_context:
        if not os.environ.get("OLLAMA_API_KEY") and "OPENAI_API_KEY" not in os.environ:
            sys.exit("Ablation B needs a provider key (OLLAMA_API_KEY).")
        print("== Ablation B: linter IR-context (with vs without IR) ==", file=sys.stderr)
        rows_b = ir_context_ablation(binary, prompts, args.model, args.timeout)
        write_csv(rows_b, HERE / "results" / "ablation_ir_context.csv")
        if rows_b:
            mw = sum(r["diagnostics_with_ir"] for r in rows_b) / len(rows_b)
            mo = sum(r["diagnostics_without_ir"] for r in rows_b) / len(rows_b)
            print(f"Mean diagnostics: with_ir={mw:.2f}  without_ir={mo:.2f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
