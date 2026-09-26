#!/usr/bin/env python3
"""Does the model ranking transfer from transcription to recovery?

Our main panel scores models on *annotated* prompts, where most of the work is
copying structure that is already marked. The separate unannotated campaign
scores them on real `SKILL.md` files, where nothing is marked and the model must
decide for itself what the units of structure are. Those are different tasks,
and a reader choosing a model from the first table is implicitly assuming the
ranking transfers to the second.

This script tests that assumption directly: it lines up the models measured in
both regimes and reports the rank correlation between them, plus the per-model
movement. A high correlation would mean the cheap annotated benchmark is a
usable proxy. A low or negative one would mean it is not, which matters more,
because the annotated benchmark is the one that is cheap to run and therefore
the one that gets published.

Inputs are `results/ir_stability_merged.csv` (annotated, paid-tier-first merge)
and `results/unannotated_stability.csv` (unannotated campaign). Runs offline.

Usage:
    python3 scripts/regime_comparison.py
"""
from __future__ import annotations

import argparse
import csv
import os
import statistics as st
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

try:
    from scipy import stats as sps
except ImportError:  # pragma: no cover
    sps = None


def read(path: str) -> list[dict]:
    if not os.path.exists(path):
        return []
    with open(path, newline="") as fh:
        return list(csv.DictReader(fh))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--annotated", default=os.path.join(HERE, "results", "ir_stability_merged.csv"))
    ap.add_argument("--unannotated", default=os.path.join(HERE, "results", "unannotated_stability.csv"))
    ap.add_argument("--metric", default="jaccard_normalised",
                    choices=("jaccard_exact", "jaccard_normalised", "count_agreement"),
                    help="which unannotated metric to rank on; normalised is the "
                         "fairest comparison because the annotated regime has "
                         "canonical ids and the unannotated one does not")
    ap.add_argument("--out", default=os.path.join(HERE, "results", "regime_comparison.csv"))
    args = ap.parse_args()

    ann = {r["model"]: float(r["mean_jaccard"]) for r in read(args.annotated)}
    ann_n = {r["model"]: int(r["n_prompts"]) for r in read(args.annotated)}

    un_rows = read(args.unannotated)
    if not un_rows:
        raise SystemExit("no unannotated results yet")
    by_model = defaultdict(list)
    for r in un_rows:
        by_model[r["model"]].append(r)
    un = {m: st.mean(float(x[args.metric]) for x in rs) for m, rs in by_model.items()}
    un_n = {m: len(rs) for m, rs in by_model.items()}

    common = sorted(set(ann) & set(un), key=lambda m: -ann[m])
    if not common:
        print("no models measured in both regimes yet")
        print(f"  annotated:   {sorted(ann)}")
        print(f"  unannotated: {sorted(un)}")
        return

    ann_rank = {m: i + 1 for i, m in enumerate(sorted(common, key=lambda m: -ann[m]))}
    un_rank = {m: i + 1 for i, m in enumerate(sorted(common, key=lambda m: -un[m]))}

    rows = []
    hdr = (f"{'model':24}{'ann n':>7}{'ann J':>8}{'rank':>6}"
           f"{'unann n':>9}{'unann':>8}{'rank':>6}{'move':>7}")
    print(f"comparing on unannotated metric: {args.metric}")
    print(hdr)
    print("-" * len(hdr))
    for m in common:
        move = ann_rank[m] - un_rank[m]
        rows.append({
            "model": m, "annotated_n": ann_n.get(m, 0), "annotated_jaccard": round(ann[m], 4),
            "annotated_rank": ann_rank[m], "unannotated_cells": un_n[m],
            "unannotated_score": round(un[m], 4), "unannotated_rank": un_rank[m],
            "rank_movement": move,
        })
        print(f"{m:24}{ann_n.get(m,0):>7}{ann[m]:>8.2f}{ann_rank[m]:>6}"
              f"{un_n[m]:>9}{un[m]:>8.2f}{un_rank[m]:>6}{move:>+7}")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    print("-" * len(hdr))
    if len(common) >= 3 and sps is not None:
        a = [ann[m] for m in common]
        u = [un[m] for m in common]
        rho, p = sps.spearmanr(a, u)
        r, pr = sps.pearsonr(a, u)
        print(f"models in both regimes: {len(common)}")
        print(f"Spearman rank correlation: rho = {rho:+.3f} (p = {p:.4f})")
        print(f"Pearson correlation:       r   = {r:+.3f} (p = {pr:.4f})")
        moved = sum(1 for x in rows if x["rank_movement"] != 0)
        print(f"models changing rank between regimes: {moved}/{len(common)}")
        biggest = max(rows, key=lambda x: abs(x["rank_movement"]))
        print(f"largest movement: {biggest['model']} "
              f"{biggest['annotated_rank']} -> {biggest['unannotated_rank']}")
    else:
        print(f"only {len(common)} models in both regimes; correlation not computed")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
