#!/usr/bin/env python3
"""Does reproducibility track correctness? (RQ7), across every candidate metric.

The manuscript reports a single correlation between per-model stability and
per-model correctness. Which correctness metric you pick changes the answer
materially, so reporting one number without naming it is exactly the
analysis-choice sensitivity this paper criticises elsewhere. This script
computes the correlation under all four, so the choice is visible.

Stability is `mean_jaccard` from the merged annotated panel, prompt-weighted.
Correctness is averaged over persisted runs from `ir_correctness.csv`:

  recall              share of gold nodes recovered
  precision_all       share of emitted nodes in the gold set. This penalises
                      correctly-inferred implicit nodes, which the instruction
                      explicitly asks for, so it is not an accuracy measure.
  precision_explicit  precision restricted to nodes the model marked explicit,
                      which is the fair precision here
  f1                  harmonic mean of recall and precision_all, and therefore
                      inherits precision_all's bias

We also report the drop-one correlation without `gpt-oss:120b`, the low outlier
that carries most of the linear signal.

Runs offline. Usage:
    python3 scripts/stability_vs_correctness.py
"""
from __future__ import annotations

import argparse
import csv
import os
import statistics as st
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
METRICS = ("recall", "precision_all", "precision_explicit", "f1")

try:
    from scipy import stats as sps
except ImportError:  # pragma: no cover
    sps = None


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--correctness", default=os.path.join(HERE, "results", "ir_correctness.csv"))
    ap.add_argument("--stability", default=os.path.join(HERE, "results", "ir_stability_merged.csv"))
    ap.add_argument("--out", default=os.path.join(HERE, "results", "stability_vs_correctness.csv"))
    ap.add_argument("--drop", default="gpt-oss:120b", help="model to omit for the drop-one check")
    args = ap.parse_args()

    runs = list(csv.DictReader(open(args.correctness)))
    by = defaultdict(list)
    for r in runs:
        by[r["model"]].append(r)
    agg = {m: {k: st.mean(float(x[k]) for x in rs) for k in METRICS} for m, rs in by.items()}
    stab = {r["model"]: float(r["mean_jaccard"]) for r in csv.DictReader(open(args.stability))}
    common = sorted(set(agg) & set(stab))

    cells = defaultdict(int)
    for r in runs:
        if r["model"] in common:
            cells[(r["model"], r["prompt"])] += 1
    singles = sum(1 for v in cells.values() if v == 1)
    print(f"runs: {len(runs)}; models with both measurements: {len(common)}")
    print(f"correctness cells for those models: {len(cells)} "
          f"({len(cells)-singles} with >=2 runs, {singles} singletons -- "
          f"all are used, since correctness needs only one run)")

    print(f"\n{'model':24}{'stability':>11}" + "".join(f"{m:>20}" for m in METRICS))
    for m in common:
        print(f"{m:24}{stab[m]:>11.3f}" + "".join(f"{agg[m][k]:>20.3f}" for k in METRICS))

    if sps is None:
        raise SystemExit("scipy required")

    xs = [stab[m] for m in common]
    kept = [m for m in common if m != args.drop]
    rows = []
    print(f"\n{'metric':20}{'pearson r':>11}{'p':>9}{'spearman':>11}{'p':>9}"
          f"{'drop-one r':>12}{'p':>9}")
    for k in METRICS:
        ys = [agg[m][k] for m in common]
        r, pr = sps.pearsonr(xs, ys)
        rho, prho = sps.spearmanr(xs, ys)
        r2, pr2 = sps.pearsonr([stab[m] for m in kept], [agg[m][k] for m in kept])
        rows.append({"metric": k, "n": len(common), "pearson_r": round(r, 4),
                     "pearson_p": round(pr, 4), "spearman_rho": round(rho, 4),
                     "spearman_p": round(prho, 4), "drop_one_r": round(r2, 4),
                     "drop_one_p": round(pr2, 4), "dropped": args.drop})
        print(f"{k:20}{r:>+11.3f}{pr:>9.4f}{rho:>+11.3f}{prho:>9.4f}{r2:>+12.3f}{pr2:>9.4f}")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    sig = [r["metric"] for r in rows if r["pearson_p"] <= 0.05]
    rob = [r["metric"] for r in rows if r["drop_one_p"] <= 0.05]
    print(f"\nsignificant at 0.05 before dropping: {sig or 'none'}")
    print(f"still significant after dropping {args.drop}: {rob or 'none'}")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
