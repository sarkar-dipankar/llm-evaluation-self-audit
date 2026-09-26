#!/usr/bin/env python3
"""RQ2: test-suite adequacy of coverage-directed context generation.

For each prompt, compares mutation score (deterministic / provider-free) under:

  - single    : ONE input (the first generated context) — a naive single-input suite
  - generated : the FULL coverage-directed set from `promptdbg gen-contexts`

Both always render (they are real generated contexts), so this isolates the value
of generating MULTIPLE branch-covering inputs over a single arbitrary input. A
higher mutation score = a more adequate suite (distinguishes more injected faults).

  python scripts/rq2_baseline.py --bin tool/target/release/promptdbg
"""
from __future__ import annotations

import argparse
import csv
import json
import statistics as st
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"


def mutation_score(binary, prompt: Path, contexts: Path):
    out = subprocess.run(
        [binary, "mutate", str(prompt), "--engine", "annotation",
         "--contexts", str(contexts), "--format", "json"],
        capture_output=True, text=True, timeout=120,
    )
    if out.returncode != 0 or not out.stdout.strip():
        return None
    try:
        d = json.loads(out.stdout)
        return d["mutation_score"], d["total_mutants"]
    except (json.JSONDecodeError, KeyError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--contexts-dir", default=str(HERE / "corpus" / "contexts"))
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--out", default=str(HERE / "results" / "rq2_baseline.csv"))
    args = ap.parse_args()

    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    ctx_dir = Path(args.contexts_dir)
    prompts = sorted(Path(args.corpus).glob("*.rtpl"))
    tmp = Path(tempfile.mkdtemp(prefix="rq2_"))

    rows = []
    for p in prompts:
        stem = p.name[:-len(".prompt.rtpl")] if p.name.endswith(".prompt.rtpl") else p.stem
        gen = ctx_dir / f"{stem}.contexts.json"
        if not gen.exists():
            continue
        ctxs = json.loads(gen.read_text())
        if not ctxs:
            continue
        single = tmp / f"{stem}.single.json"
        single.write_text(json.dumps([ctxs[0]]))

        full_m = mutation_score(binary, p, gen)
        single_m = mutation_score(binary, p, single)
        if full_m is None or single_m is None:
            print(f"  {p.name}: unrenderable, skipping", file=sys.stderr)
            continue
        rows.append({
            "prompt": p.name,
            "total_mutants": full_m[1],
            "n_contexts": len(ctxs),
            "score_single": round(single_m[0], 3),
            "score_generated": round(full_m[0], 3),
            "gain": round(full_m[0] - single_m[0], 3),
        })
        print(f"  {p.name}: single={rows[-1]['score_single']} "
              f"generated={rows[-1]['score_generated']} gain={rows[-1]['gain']}",
              file=sys.stderr)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    fields = ["prompt", "total_mutants", "n_contexts", "score_single", "score_generated", "gain"]
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    print(f"\nWrote {len(rows)} rows to {out}")
    if rows:
        ms = st.mean(r["score_single"] for r in rows)
        mg = st.mean(r["score_generated"] for r in rows)
        print(f"Mean mutation score: single-input={ms:.3f}  coverage-directed={mg:.3f}  (+{mg - ms:.3f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
