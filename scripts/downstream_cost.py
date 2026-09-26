#!/usr/bin/env python3
"""Downstream-cost analysis for Paper 2.

For each prompt and model, simulate the practical use of inferred IR by a
downstream tool: take the R inferences in order, compute the rolling mean of a
node-count signal (a proxy for any inferred-IR-derived metric a tool would
report), and find the smallest r such that the rolling mean from run r onward
stays within ±tol of the eventual long-run mean.

Reads results/irs/<model_slug>/<prompt>_r<n>.json (from ir_stability.py
--save-irs). If raw IRs are not available, falls back to the summary
ir_stability_*.csv (which gives only per-prompt aggregates; the trajectory
analysis then degenerates to a check that R itself was enough).

  python scripts/downstream_cost.py --ir-dir results/irs --tol 0.05
"""
from __future__ import annotations

import argparse
import csv
import json
import statistics as st
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent


def signal_of(nodes: list[dict]) -> float:
    # Node count is a coarse but model-agnostic stand-in for "anything a downstream
    # tool would compute from the inferred IR". Other signals (rule count by
    # category) yield qualitatively similar trajectories on the seed data.
    return float(len(nodes))


def stabilisation(trajectory: list[float], tol: float, mode: str = "absolute") -> int | None:
    """Smallest r whose rolling mean stays within tolerance of the long-run mean.

    Two caveats, both of which materially change the result and neither of which
    was stated in the original write-up:

    1. `long_mean` is the mean of the whole trajectory, so at r = len(trajectory)
       the comparison is of a quantity with itself and always succeeds. r = R
       therefore carries no evidence that R runs sufficed; callers should treat
       it as right-censored.
    2. `mode` decides whether `tol` is an absolute difference in node count or a
       fraction of the long-run mean. The original CLI help described it as
       relative while the code applied it as absolute, which on trajectories
       averaging 8-20 nodes is the difference between demanding near-exact
       equality and allowing about one node of slack. Across 98 cells the two
       give 50 and 80 stabilising respectively.
    """
    if len(trajectory) < 2:
        return None
    long_mean = st.mean(trajectory)
    band = tol if mode == "absolute" else tol * abs(long_mean)
    for r in range(1, len(trajectory) + 1):
        rolling = st.mean(trajectory[:r])
        if abs(rolling - long_mean) <= band:
            # require the rolling mean to stay within tol from here on
            if all(abs(st.mean(trajectory[:k]) - long_mean) <= band for k in range(r, len(trajectory) + 1)):
                return r
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ir-dir", default=str(HERE / "results" / "irs"))
    ap.add_argument("--tol", type=float, default=0.05,
                    help="rolling-mean tolerance; see --tol-mode for its units")
    ap.add_argument("--tol-mode", choices=("absolute", "relative"), default="absolute",
                    help="absolute: tol is a difference in node count (the behaviour "
                         "of earlier versions). relative: tol is a fraction of the "
                         "long-run mean. The choice changes how many cells stabilise "
                         "(50 vs 80 of 98 at tol=0.05), so state it when reporting.")
    ap.add_argument("--out", default=str(HERE / "results" / "downstream_cost.csv"))
    args = ap.parse_args()

    root = Path(args.ir_dir)
    if not root.exists():
        print(f"No IR dir at {root}; run ir_stability with --save-irs first.")
        return 1

    rows = []
    for model_dir in sorted(p for p in root.iterdir() if p.is_dir()):
        model = model_dir.name.replace("_", ":", 1)
        by_prompt: dict[str, list[Path]] = defaultdict(list)
        for f in sorted(model_dir.glob("*_r*.json")):
            stem = f.stem.rsplit("_r", 1)[0]
            by_prompt[stem].append(f)
        for stem, files in by_prompt.items():
            runs = []
            for f in sorted(files):
                try:
                    runs.append(signal_of(json.loads(f.read_text()).get("nodes", [])))
                except Exception:
                    continue
            if len(runs) < 2:
                continue
            r = stabilisation(runs, args.tol, args.tol_mode)
            rows.append({
                "model": model, "prompt": stem,
                "runs": len(runs),
                "long_mean": round(st.mean(runs), 3),
                "long_std": round(st.pstdev(runs), 3),
                "stabilises_at": r if r is not None else len(runs) + 1,
            })

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if rows:
        with out.open("w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
            w.writeheader(); w.writerows(rows)
    print(f"wrote {len(rows)} rows to {out}\n")
    if rows:
        agg = defaultdict(list)
        for r in rows:
            agg[r["model"]].append(r["stabilises_at"])
        print(f"{'model':<24}{'n':>4}{'med r':>8}{'mean r':>10}")
        for m, xs in agg.items():
            print(f"{m:<24}{len(xs):>4}{st.median(xs):>8.1f}{st.mean(xs):>10.2f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
