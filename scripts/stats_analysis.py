#!/usr/bin/env python3
"""Every inferential number in Paper 2, computed in one auditable place.

The manuscript previously reported a Wilcoxon p-value, bootstrap confidence
intervals, a drift summary row, and a stabilisation point, none of which was
produced by a committed script. This module produces all of them, states the
estimand for each, and reports how sensitive each is to the analysis choices
that were previously left implicit.

Sections:
  1. Per-model stability with bootstrap CIs over prompts.
  2. Rank stability under prompt resampling.
  3. Within-family size contrasts (the "bigger is not better" claim), with the
     prompt set and tie handling stated explicitly.
  4. Drift composition, under every defensible aggregation.
  5. Stabilisation point with right-censoring.
  6. Sensitivity of the headline table to the merge precedence rule.

Usage:
    python3 scripts/stats_analysis.py [--boot 10000] [--seed 20260817]
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import random
import statistics as st
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RESULTS = os.path.join(HERE, "results")

try:
    from scipy import stats as sps
except ImportError:  # pragma: no cover
    sps = None


def read(name: str) -> list[dict]:
    path = os.path.join(RESULTS, name)
    if not os.path.exists(path):
        return []
    with open(path, newline="") as fh:
        return list(csv.DictReader(fh))


# ---------------------------------------------------------------- 1. bootstrap

def bootstrap_ci(values: list[float], n_boot: int, rng: random.Random, alpha=0.05):
    """Percentile bootstrap CI for the mean, resampling prompts."""
    if len(values) < 2:
        return (float("nan"), float("nan"))
    means = []
    n = len(values)
    for _ in range(n_boot):
        means.append(st.mean(rng.choices(values, k=n)))
    means.sort()
    lo = means[int((alpha / 2) * n_boot)]
    hi = means[min(n_boot - 1, int((1 - alpha / 2) * n_boot))]
    return (lo, hi)


def per_model_stability(cells: list[dict], n_boot: int, rng: random.Random):
    by_model = defaultdict(list)
    for r in cells:
        by_model[r["model"]].append(r)

    out = []
    for model, rows in by_model.items():
        j = [float(r["mean_jaccard"]) for r in rows]
        i = [float(r["id_stability"]) for r in rows]
        jlo, jhi = bootstrap_ci(j, n_boot, rng)
        ilo, ihi = bootstrap_ci(i, n_boot, rng)
        perfect = sum(1 for r in rows if int(r["perfect"]))
        out.append({
            "model": model, "n": len(rows),
            "jaccard": st.mean(j), "j_lo": jlo, "j_hi": jhi,
            "id": st.mean(i), "id_lo": ilo, "id_hi": ihi,
            "perfect": perfect,
        })
    out.sort(key=lambda d: -d["jaccard"])
    return out


# ------------------------------------------------------------ 2. rank stability

def rank_stability(cells: list[dict], n_boot: int, rng: random.Random):
    """How often does each model keep its rank when prompts are resampled?

    The resampling is a *joint* cluster bootstrap over prompts: one prompt
    multiset is drawn per replicate and every model is then scored on that same
    multiset. Resampling each model's values independently would compare models
    on different pseudo-prompts and destroy the paired design, which inflates
    apparent instability for models measured on overlapping prompt sets.

    A model contributes to a replicate only via the drawn prompts it actually
    has data for; a replicate in which some model has no observation at all is
    discarded, and the number discarded is reported.
    """
    by_model = defaultdict(dict)          # model -> prompt -> jaccard
    for r in cells:
        by_model[r["model"]][r["prompt"]] = float(r["mean_jaccard"])
    prompts = sorted({r["prompt"] for r in cells})

    models = sorted(by_model, key=lambda m: -st.mean(list(by_model[m].values())))
    observed = {m: k for k, m in enumerate(models)}
    keeps = defaultdict(int)
    used = 0
    for _ in range(n_boot):
        draw_prompts = rng.choices(prompts, k=len(prompts))
        scores = {}
        ok = True
        for m in models:
            vals = [by_model[m][p] for p in draw_prompts if p in by_model[m]]
            if not vals:
                ok = False
                break
            scores[m] = st.mean(vals)
        if not ok:
            continue
        used += 1
        order = sorted(scores, key=lambda m: -scores[m])
        for k, m in enumerate(order):
            if k == observed[m]:
                keeps[m] += 1
    return ([(m, observed[m] + 1, keeps[m] / used) for m in models], used, n_boot - used)


# --------------------------------------------------------- 3. size contrasts

def paired_contrast(cells: list[dict], small: str, large: str):
    """Paired within-family contrast on the prompts both models completed."""
    by = defaultdict(dict)
    for r in cells:
        by[r["prompt"]][r["model"]] = float(r["mean_jaccard"])
    common = sorted(p for p, d in by.items() if small in d and large in d)
    if not common:
        return None
    xs = [by[p][large] for p in common]
    ys = [by[p][small] for p in common]
    diffs = [a - b for a, b in zip(xs, ys)]
    nonzero = [d for d in diffs if d != 0]
    res = {
        "small": small, "large": large, "n_common": len(common),
        "mean_large": st.mean(xs), "mean_small": st.mean(ys),
        "mean_diff": st.mean(diffs), "n_nonzero": len(nonzero),
    }
    if sps is not None and len(nonzero) >= 1:
        # Report both tie conventions, because they differ materially here and
        # the manuscript never said which was used.
        try:
            res["p_wilcox_zsplit"] = float(sps.wilcoxon(xs, ys, zero_method="zsplit").pvalue)
        except ValueError:
            res["p_wilcox_zsplit"] = float("nan")
        try:
            res["p_wilcox_drop"] = float(sps.wilcoxon(xs, ys, zero_method="wilcox").pvalue)
        except ValueError:
            res["p_wilcox_drop"] = float("nan")
        # Matched-pairs rank-biserial from the signed-rank sums, which is the
        # statistic conventionally paired with a Wilcoxon signed-rank test. The
        # sign-count version below is a different quantity and was previously
        # written out under this name; both are reported so neither is confused
        # for the other.
        pos = sum(1 for d in nonzero if d > 0)
        res["sign_biserial"] = (2.0 * pos / len(nonzero)) - 1.0 if nonzero else float("nan")
        ranks = sps.rankdata([abs(d) for d in nonzero])
        r_pos = sum(rk for rk, d in zip(ranks, nonzero) if d > 0)
        r_neg = sum(rk for rk, d in zip(ranks, nonzero) if d < 0)
        total = r_pos + r_neg
        res["rank_biserial"] = (r_pos - r_neg) / total if total else float("nan")
    return res


# ------------------------------------------------------------ 4. drift summary

def drift_summary(rows: list[dict], label: str):
    macro = tuple(
        100 * st.mean(float(r[k]) for r in rows)
        for k in ("frac_id", "frac_count", "frac_hier")
    )
    i = sum(int(r["id_drift"]) for r in rows)
    c = sum(int(r["count_drift"]) for r in rows)
    h = sum(int(r["hierarchy_drift"]) for r in rows)
    tot = i + c + h
    pooled = (100 * i / tot, 100 * c / tot, 100 * h / tot) if tot else (0.0, 0.0, 0.0)
    dis = [r for r in rows
           if int(r["id_drift"]) + int(r["count_drift"]) + int(r["hierarchy_drift"]) > 0]
    cond = tuple(
        100 * st.mean(float(r[k]) for r in dis)
        for k in ("frac_id", "frac_count", "frac_hier")
    ) if dis else (0.0, 0.0, 0.0)
    return {"label": label, "cells": len(rows), "disagreeing": len(dis),
            "macro_all": macro, "pooled_pairs": pooled, "macro_disagreeing": cond}


# ------------------------------------------------- 5. right-censored stabilisation

def stabilisation_censored(rows: list[dict]):
    """Split cells into genuinely-stabilised and right-censored.

    The original metric compared the rolling mean at r to the mean of the whole
    trajectory. At r = R those are by definition the same number, so r = R
    always 'passes' and the never-stabilised outcome cannot occur. A cell that
    only reaches tolerance at its own endpoint carries no evidence that R runs
    were sufficient, so it is right-censored rather than counted as r = R.
    """
    stabilised = [r for r in rows if int(r["stabilises_at"]) < int(r["runs"])]
    censored = [r for r in rows if int(r["stabilises_at"]) >= int(r["runs"])]
    by_model = defaultdict(lambda: {"stab": [], "cens": 0})
    for r in rows:
        m = r["model"]
        if int(r["stabilises_at"]) < int(r["runs"]):
            by_model[m]["stab"].append(int(r["stabilises_at"]))
        else:
            by_model[m]["cens"] += 1
    return stabilised, censored, by_model


# ------------------------------------------------- 6. merge-rule sensitivity

# Highest precedence first under each rule. "Paid-first" prefers the
# higher-budget campaign; "earliest-first" takes the oldest campaign and tops up
# from later ones only for prompts it did not cover, which is what an analyst
# assembling results incrementally would naturally do.
PAID_FIRST = [
    "ir_stability_paid_tierD.csv", "ir_stability_paid_tierC.csv",
    "ir_stability_paid_tierB.csv", "ir_stability_scaled.csv",
    "ir_stability_combined.csv", "ir_stability_qwen.csv",
    "ir_stability_ministral.csv", "ir_stability_minimax.csv",
    "ir_stability_xmodel.csv", "ir_stability.csv",
]
EARLIEST_FIRST = [
    "ir_stability.csv", "ir_stability_xmodel.csv", "ir_stability_combined.csv",
    "ir_stability_minimax.csv", "ir_stability_ministral.csv",
    "ir_stability_qwen.csv", "ir_stability_scaled.csv",
    "ir_stability_paid_tierB.csv", "ir_stability_paid_tierC.csv",
    "ir_stability_paid_tierD.csv",
]


def _merge_under(order: list[str]) -> dict[str, dict]:
    best: dict[tuple[str, str], dict] = {}
    for name in order:
        for row in read(name):
            key = (row["model"], row["prompt"])
            if key not in best:
                best[key] = row
    by_model = defaultdict(list)
    for (model, _p), row in best.items():
        by_model[model].append(row)
    out = {}
    for model, rows in by_model.items():
        out[model] = {
            "n": len(rows),
            "jaccard": st.mean(float(r["mean_jaccard"]) for r in rows),
            "id": st.mean(float(r["id_stability"]) for r in rows),
            "perfect": sum(
                1 for r in rows
                if float(r["mean_jaccard"]) >= 1.0 - 1e-9
                and float(r["id_stability"]) >= 1.0 - 1e-9
            ),
        }
    return out


def merge_sensitivity() -> dict:
    """Recompute the whole table under both rules. Never hard-code this."""
    a, b = _merge_under(PAID_FIRST), _merge_under(EARLIEST_FIRST)
    rows = []
    for model in sorted(a, key=lambda m: -a[m]["jaccard"]):
        x, y = a[model], b[model]
        differs = (
            abs(x["jaccard"] - y["jaccard"]) > 1e-3
            or abs(x["id"] - y["id"]) > 1e-3
            or x["perfect"] != y["perfect"]
        )
        rows.append({"model": model, "paid_first": x, "earliest_first": y,
                     "differs": differs})
    pp = sum(v["perfect"] for v in a.values())
    pe = sum(v["perfect"] for v in b.values())
    total = sum(v["n"] for v in a.values())
    return {
        "rows": rows,
        "n_differing": sum(1 for r in rows if r["differs"]),
        "perfect_paid": pp, "perfect_earliest": pe, "total": total,
        "pct_paid": 100.0 * pp / total, "pct_earliest": 100.0 * pe / total,
    }


# ---------------------------------------------------------------------- report

def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--boot", type=int, default=10000)
    ap.add_argument("--seed", type=int, default=20260817)
    ap.add_argument("--out", default=os.path.join(RESULTS, "stats_analysis.json"))
    args = ap.parse_args()
    rng = random.Random(args.seed)

    cells = read("ir_stability_merged_cells.csv")
    if not cells:
        raise SystemExit("run scripts/merge_ir_stability.py first")

    report: dict = {"seed": args.seed, "n_boot": args.boot}

    print("=" * 74)
    print("1. Per-model stability, 95% percentile bootstrap over prompts")
    print("=" * 74)
    tbl = per_model_stability(cells, args.boot, rng)
    print(f"{'model':24}{'n':>4}  {'Jaccard [95% CI]':<26}{'ID-stability [95% CI]':<26}{'perfect':>9}")
    for r in tbl:
        j = f"{r['jaccard']:.2f} [{r['j_lo']:.2f}, {r['j_hi']:.2f}]"
        i = f"{r['id']:.2f} [{r['id_lo']:.2f}, {r['id_hi']:.2f}]"
        perfect = f"{r['perfect']}/{r['n']}"
        print(f"{r['model']:24}{r['n']:>4}  {j:<26}{i:<26}{perfect:>9}")
    report["per_model"] = tbl

    print()
    print("=" * 74)
    print("2. Rank stability under prompt resampling")
    print("=" * 74)
    rs, used, dropped = rank_stability(cells, args.boot, rng)
    print(f"  joint cluster bootstrap over prompts: {used} usable replicates, "
          f"{dropped} discarded (a model had no drawn prompt)")
    for m, rank, keep in rs:
        print(f"  {m:24} observed rank {rank}  retained in {keep*100:5.1f}% of replicates")
    report["rank_stability"] = [{"model": m, "rank": r, "retained": k} for m, r, k in rs]
    report["rank_bootstrap_used"] = used
    report["rank_bootstrap_dropped"] = dropped

    print()
    print("=" * 74)
    print("3. Within-family size contrasts")
    print("=" * 74)
    pairs = [("gpt-oss:20b", "gpt-oss:120b"),
             ("ministral-3:8b", "mistral-large-3:675b"),
             ("minimax-m2.1", "minimax-m2.7")]
    contrasts = []
    for small, large in pairs:
        c = paired_contrast(cells, small, large)
        if not c:
            print(f"  {small} vs {large}: no common prompts")
            continue
        contrasts.append(c)
        print(f"  {large} vs {small}: n_common={c['n_common']} "
              f"(nonzero {c['n_nonzero']})")
        print(f"    mean J large={c['mean_large']:.3f} small={c['mean_small']:.3f} "
              f"diff={c['mean_diff']:+.3f}")
        if "p_wilcox_zsplit" in c:
            print(f"    Wilcoxon p: zsplit={c['p_wilcox_zsplit']:.5f} "
                  f"drop-zeros={c['p_wilcox_drop']:.5f}  "
                  f"rank-biserial={c['rank_biserial']:+.3f}")
    report["size_contrasts"] = contrasts

    print()
    print("=" * 74)
    print("4. Drift composition (id / count / hierarchy), every aggregation")
    print("=" * 74)
    drift = read("drift_decompose.csv")
    summaries = []
    for label, rows in (("all cells", drift),
                        ("excluding glm-4.7", [r for r in drift if r["model"] != "glm-4.7"])):
        s = drift_summary(rows, label)
        summaries.append(s)
        print(f"  {label} (n={s['cells']}, {s['disagreeing']} disagreeing)")
        for k in ("macro_all", "pooled_pairs", "macro_disagreeing"):
            a, b, c = s[k]
            print(f"    {k:20} id={a:5.2f}  count={b:5.2f}  hier={c:5.2f}")
    report["drift"] = summaries

    print()
    print("=" * 74)
    print("5. Stabilisation point, right-censored")
    print("=" * 74)
    dc = read("downstream_cost.csv")
    stab, cens, by_model = stabilisation_censored(dc)
    print(f"  cells: {len(dc)};  genuinely stabilised before the endpoint: {len(stab)};"
          f"  right-censored at r=R: {len(cens)} ({100*len(cens)/len(dc):.0f}%)")
    print(f"  {'model':24}{'stabilised':>11}{'censored':>10}{'median r (stabilised)':>24}")
    cens_tbl = []
    for m in sorted(by_model):
        d = by_model[m]
        med = st.median(d["stab"]) if d["stab"] else float("nan")
        print(f"  {m:24}{len(d['stab']):>11}{d['cens']:>10}{med:>24.1f}")
        cens_tbl.append({"model": m, "stabilised": len(d["stab"]),
                         "censored": d["cens"],
                         "median_r": None if not d["stab"] else med})
    report["stabilisation"] = {"total": len(dc), "stabilised": len(stab),
                               "censored": len(cens), "by_model": cens_tbl}

    print()
    print("=" * 74)
    print("6. Sensitivity of the headline table to the merge precedence rule")
    print("=" * 74)
    sens = merge_sensitivity()
    hdr = f"{'model':24}{'paid-first':>26}{'earliest-first':>26}  differs"
    print(hdr)
    print("-" * len(hdr))
    for row in sens["rows"]:
        a, b = row["paid_first"], row["earliest_first"]
        fa = f"({a['n']},{a['jaccard']:.3f},{a['id']:.3f},{a['perfect']})"
        fb = f"({b['n']},{b['jaccard']:.3f},{b['id']:.3f},{b['perfect']})"
        print(f"{row['model']:24}{fa:>26}{fb:>26}  {'YES' if row['differs'] else 'no'}")
    print("-" * len(hdr))
    print(f"  models whose row changes: {sens['n_differing']}/{len(sens['rows'])}")
    print(f"  perfectly-stable cells, paid-first:     "
          f"{sens['perfect_paid']}/{sens['total']} ({sens['pct_paid']:.1f}%)")
    print(f"  perfectly-stable cells, earliest-first: "
          f"{sens['perfect_earliest']}/{sens['total']} ({sens['pct_earliest']:.1f}%)")
    print(f"  headline moves by {abs(sens['pct_paid'] - sens['pct_earliest']):.1f} percentage points")
    report["merge_sensitivity"] = sens

    with open(args.out, "w") as fh:
        json.dump(report, fh, indent=2, default=float)
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
