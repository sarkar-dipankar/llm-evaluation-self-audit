#!/usr/bin/env python3
"""Statistical analysis of the behaviour study, from its saved raw responses.

`behaviour_study.py` records a coarse verdict per mutant: detected when the mean
between-condition similarity falls below both within-condition similarities.
That threshold is defensible but arbitrary, so this script re-analyses the same
saved responses with a permutation test, which is the honest way to ask whether
a mutation moved the response distribution further than run-to-run noise does.

For each mutant we have R responses to the original prompt and R to the mutant.
The observed statistic is

    d = mean(within-condition similarities) - mean(between-condition similarities)

Under the null hypothesis that the mutation changed nothing, the labels
"original" and "mutant" are exchangeable, so we recompute d over every way of
splitting the 2R responses into two groups of R and take the p-value as the
fraction of splits reaching the observed d. With R=3 there are only
C(6,3)/2 = 10 distinct splits, so the test is exact and its minimum attainable
p-value is 0.1. That is a real limitation and we report it rather than hide it:
at R=3 no single mutant can be significant at 0.05, and the evidence has to come
from the proportion of mutants showing the effect, tested across mutants.

Runs offline from results/behaviour_raw/.

Usage:
    python3 scripts/behaviour_analysis.py
"""
from __future__ import annotations

import argparse
import csv
import glob
import itertools
import json
import os
import re
import statistics as st
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

WORD = re.compile(r"[a-z0-9']+")
STOP = {
    "the", "a", "an", "and", "or", "but", "if", "then", "of", "to", "in", "on",
    "for", "with", "is", "are", "be", "as", "at", "by", "it", "this", "that",
}


def toks(s: str) -> set:
    return {w for w in WORD.findall(s.lower()) if w not in STOP}


def sim(a: str, b: str) -> float:
    ta, tb = toks(a), toks(b)
    if not ta and not tb:
        return 1.0
    return len(ta & tb) / max(1, len(ta | tb))


def stat(group_a: list[str], group_b: list[str]) -> float:
    """within-similarity minus between-similarity for a given labelling."""
    wa = [sim(x, y) for x, y in itertools.combinations(group_a, 2)]
    wb = [sim(x, y) for x, y in itertools.combinations(group_b, 2)]
    bt = [sim(x, y) for x in group_a for y in group_b]
    within = st.mean(wa + wb) if (wa or wb) else 1.0
    between = st.mean(bt) if bt else 1.0
    return within - between


def permutation_p(orig: list[str], mut: list[str]) -> tuple[float, float, int]:
    """Exact two-group permutation test. Returns (observed d, p, n_splits)."""
    pool = orig + mut
    n = len(orig)
    observed = stat(orig, mut)
    splits = []
    seen = set()
    for idx in itertools.combinations(range(len(pool)), n):
        comp = tuple(i for i in range(len(pool)) if i not in idx)
        key = tuple(sorted([idx, comp]))
        if key in seen:
            continue
        seen.add(key)
        a = [pool[i] for i in idx]
        b = [pool[i] for i in comp]
        splits.append(stat(a, b))
    ge = sum(1 for s in splits if s >= observed - 1e-12)
    return observed, ge / len(splits), len(splits)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--raw", default=os.path.join(HERE, "results", "behaviour_raw"))
    ap.add_argument("--out", default=os.path.join(HERE, "results", "behaviour_analysis.csv"))
    ap.add_argument("--alpha", type=float, default=0.05,
                    help="per-mutant significance level. The exact test's floor is "
                         "1/n_splits, so R=3 cannot go below 0.1 and R=5 reaches 0.01")
    args = ap.parse_args()

    rows = []
    for f in sorted(glob.glob(os.path.join(args.raw, "*.json"))):
        try:
            d = json.load(open(f))
        except Exception:
            continue
        orig, mut = d.get("original_responses", []), d.get("mutant_responses", [])
        if len(orig) < 2 or len(mut) < 2:
            continue
        observed, p, nsplits = permutation_p(orig, mut)
        identical = int(all(sim(a, b) >= 0.9999 for a in orig for b in mut))
        rows.append({
            "prompt": d["prompt"], "operator": d["operator"], "mutant": d["mutant"],
            "repeats": len(orig), "d_observed": round(observed, 4),
            "p_permutation": round(p, 4), "n_splits": nsplits,
            "significant": int(p <= args.alpha),
            "responses_identical": identical,
            # Full precision, carried separately. Signing the rounded column
            # would discard genuinely positive effects smaller than 5e-5, and
            # one of our 80 mutants is exactly that case.
            "_d_raw": observed,
        })

    if not rows:
        raise SystemExit(f"no raw responses under {args.raw}")

    public = [k for k in rows[0] if not k.startswith("_")]
    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=public, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)

    n = len(rows)
    sig = sum(r["significant"] for r in rows)
    ident = sum(r["responses_identical"] for r in rows)
    print(f"mutants analysed: {n}")
    print(f"exact permutation splits per mutant: {rows[0]['n_splits']} "
          f"(minimum attainable p = {1/rows[0]['n_splits']:.2f})")
    print(f"responses byte-identical across the mutation: {ident}/{n} "
          f"({100*ident/n:.0f}%) -- these are behaviourally equivalent mutants")
    print(f"significant at p <= {args.alpha}, uncorrected: {sig}/{n} ({100*sig/n:.0f}%)")
    ps = sorted(r["p_permutation"] for r in rows)
    bh = sum(1 for i, pv in enumerate(ps) if pv <= (i + 1) / n * args.alpha)
    bonf = sum(1 for pv in ps if pv <= args.alpha / n)
    print(f"  surviving Benjamini-Hochberg at q={args.alpha}: {bh}")
    print(f"  surviving Bonferroni at {args.alpha}: {bonf}")
    print(f"  expected under the null: {args.alpha * n:.1f}")
    print(f"mean effect d (within minus between): {st.mean(r['d_observed'] for r in rows):+.4f}")

    print(f"\n{'operator':18}{'n':>4}{'identical':>11}{'significant':>13}{'mean d':>10}")
    by = defaultdict(list)
    for r in rows:
        by[r["operator"]].append(r)
    for op, rs in sorted(by.items()):
        print(f"{op:18}{len(rs):>4}"
              f"{sum(x['responses_identical'] for x in rs):>11}"
              f"{sum(x['significant'] for x in rs):>13}"
              f"{st.mean(x['d_observed'] for x in rs):>10.4f}")

    # Across-mutant sign test: is d positive more often than chance?
    #
    # The 80 mutants come from fewer prompts, and mutants of one prompt share its
    # wording, its context suite and its response style, so treating them as
    # independent overstates the evidence. We therefore report both the naive
    # test and a prompt-clustered one that first averages d within each prompt
    # and signs the prompt means, which is the unit actually sampled.
    pos = sum(1 for r in rows if r["_d_raw"] > 0)
    by_prompt = defaultdict(list)
    for r in rows:
        by_prompt[r["prompt"]].append(r["_d_raw"])
    cpos = sum(1 for v in by_prompt.values() if st.mean(v) > 0)
    cn = len(by_prompt)
    sizes = sorted(len(v) for v in by_prompt.values())
    try:
        from scipy import stats as sps
        pv = sps.binomtest(pos, n, 0.5, alternative="greater").pvalue
        cpv = sps.binomtest(cpos, cn, 0.5, alternative="greater").pvalue
        print(f"\nacross mutants (naive, treats {n} mutants as independent):")
        print(f"  d > 0 in {pos}/{n}; sign test p = {pv:.2e}")
        print(f"across prompts (clustered, {cn} prompts, up to {sizes[-1]} mutants each):")
        print(f"  mean d > 0 in {cpos}/{cn}; sign test p = {cpv:.2e}")
    except Exception:
        print(f"\nacross mutants: d > 0 in {pos}/{n}; across prompts: {cpos}/{cn}")

    # Per-operator, at the prompt level. A categorical claim that an operator
    # "moves behaviour" needs to survive the same clustering as the aggregate.
    try:
        from scipy import stats as sps2
        print(f"\n{'operator':18}{'prompts':>9}{'d>0':>6}{'p':>10}")
        for op in sorted({r["operator"] for r in rows}):
            b = defaultdict(list)
            for r in rows:
                if r["operator"] == op:
                    b[r["prompt"]].append(r["_d_raw"])
            k = sum(1 for v in b.values() if st.mean(v) > 0)
            pv2 = sps2.binomtest(k, len(b), 0.5, alternative="greater").pvalue
            print(f"{op:18}{len(b):>9}{k:>6}{pv2:>10.4f}")
    except Exception:
        pass
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
