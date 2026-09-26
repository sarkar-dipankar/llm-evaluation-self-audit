#!/usr/bin/env python3
"""Stability of structure *recovery* on genuinely unannotated prompts.

Our main panel measures prompts that already carry `// @rule` annotations, so it
is mostly a test of whether a model can transcribe marked structure. This script
analyses the separate campaign over unannotated real `SKILL.md` files, where
there is nothing to transcribe and the model must decide for itself what the
units of structure are.

That difference makes the headline metric unfair in a specific way, and the
script reports three metrics rather than one so the unfairness is visible:

  exact        node-set Jaccard on ids as emitted. This is what the main panel
               uses. On unannotated input it can read 0.00 for two runs that
               agree completely about content but disagree about whether a unit
               is called `SECTION_WHEN_TO_USE` or `RULE:WHEN_TO_USE`.
  normalised   Jaccard after stripping a leading `RULE:`/`SECTION:`-style
               prefix, lowercasing, and collapsing separators. Naming
               conventions no longer count as disagreement; the identity of the
               units still does.
  count        agreement on how many nodes there are, as
               1 - |n_i - n_j| / max(n_i, n_j). Independent of naming entirely.

Reporting only `exact` would overstate the instability, and reporting only
`normalised` would hide that a downstream tool keying on ids gets no stability
at all. Both are true and they answer different questions.

Usage:
    python3 scripts/unannotated_stability.py
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

PREFIX = re.compile(r"^(rule|section|condition|breakpoint|directive|node)[:_\-]", re.I)
SEP = re.compile(r"[^a-z0-9]+")


def normalise(node_id: str) -> str:
    s = node_id.strip()
    prev = None
    while prev != s:                      # strip repeated prefixes, e.g. RULE:SECTION_X
        prev = s
        s = PREFIX.sub("", s)
    return SEP.sub("_", s.lower()).strip("_")


def load(path: str):
    try:
        d = json.load(open(path))
    except Exception:
        return None
    nodes = d.get("nodes") or (d.get("ir") or {}).get("nodes") or []
    ids = [n.get("id") for n in nodes if n.get("id")]
    return ids


def jaccard(a: set, b: set) -> float:
    if not a and not b:
        return 1.0
    return len(a & b) / max(1, len(a | b))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ir-dir", default=os.path.join(HERE, "results", "irs_crossed"))
    ap.add_argument("--out", default=os.path.join(HERE, "results", "unannotated_stability.csv"))
    args = ap.parse_args()

    by_cell = defaultdict(list)
    for path in sorted(glob.glob(os.path.join(args.ir_dir, "*", "*.json"))):
        model = os.path.basename(os.path.dirname(path)).replace("_", ":", 1)
        stem = os.path.basename(path).rsplit("_r", 1)[0]
        ids = load(path)
        if ids is None:
            continue
        by_cell[(model, stem)].append(ids)

    rows = []
    for (model, prompt), runs in sorted(by_cell.items()):
        if len(runs) < 2:
            continue
        pairs = list(itertools.combinations(range(len(runs)), 2))
        ex = st.mean(jaccard(set(runs[i]), set(runs[j])) for i, j in pairs)
        nm = st.mean(
            jaccard({normalise(x) for x in runs[i]}, {normalise(x) for x in runs[j]})
            for i, j in pairs
        )
        cn = st.mean(
            1 - abs(len(runs[i]) - len(runs[j])) / max(1, max(len(runs[i]), len(runs[j])))
            for i, j in pairs
        )
        rows.append({
            "model": model, "prompt": prompt, "runs": len(runs),
            "mean_nodes": round(st.mean(len(r) for r in runs), 2),
            "jaccard_exact": round(ex, 4),
            "jaccard_normalised": round(nm, 4),
            "count_agreement": round(cn, 4),
        })

    if not rows:
        raise SystemExit(f"no comparable cells under {args.ir_dir}")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    by_model = defaultdict(list)
    for r in rows:
        by_model[r["model"]].append(r)

    hdr = f"{'model':24}{'cells':>6}{'nodes':>8}{'exact':>8}{'normalised':>12}{'count':>8}"
    print(hdr)
    print("-" * len(hdr))
    for m in sorted(by_model, key=lambda m: -st.mean(x["jaccard_normalised"] for x in by_model[m])):
        rs = by_model[m]
        print(f"{m:24}{len(rs):>6}{st.mean(x['mean_nodes'] for x in rs):>8.1f}"
              f"{st.mean(x['jaccard_exact'] for x in rs):>8.2f}"
              f"{st.mean(x['jaccard_normalised'] for x in rs):>12.2f}"
              f"{st.mean(x['count_agreement'] for x in rs):>8.2f}")
    print("-" * len(hdr))
    print(f"{'all':24}{len(rows):>6}{st.mean(x['mean_nodes'] for x in rows):>8.1f}"
          f"{st.mean(x['jaccard_exact'] for x in rows):>8.2f}"
          f"{st.mean(x['jaccard_normalised'] for x in rows):>12.2f}"
          f"{st.mean(x['count_agreement'] for x in rows):>8.2f}")

    zero = sum(1 for r in rows if r["jaccard_exact"] == 0.0)
    print(f"\ncells with exact Jaccard 0.00: {zero}/{len(rows)} "
          f"({100*zero/len(rows):.0f}%)")
    gain = st.mean(r["jaccard_normalised"] - r["jaccard_exact"] for r in rows)
    print(f"mean gain from normalising ids: {gain:+.3f} "
          "(how much of the disagreement is naming rather than substance)")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
