#!/usr/bin/env python3
"""Is the inferred IR *correct*, not merely stable?

Reproducibility is a precondition for trusting structure inference, not evidence
of it: a model can be perfectly stable and perfectly wrong. Our synthesised
corpus is annotated, and the deterministic annotation parser recovers those
annotations exactly, so it provides ground truth for the explicit part of the
structure. This script compares each persisted LLM-inferred IR against it.

For each (model, prompt, run) we compute, over node ids:

    precision  inferred ids that are real annotated ids
    recall     annotated ids the model recovered
    F1         harmonic mean

Two caveats the numbers must be read with. First, the inference prompt asks the
model to infer *additional* implicit rules and mark them `inferred=true`, so a
node absent from the annotations is not automatically an error. We therefore
report precision twice: over all inferred nodes, and over only those the model
marked explicit, where a mismatch is unambiguously wrong. Second, ground truth
covers the explicit annotations only; we make no claim about whether the
implicit rules a model proposes are good ones.

Runs entirely offline from results/irs/. No API key, no network.

Usage:
    python3 scripts/ir_correctness.py
"""
from __future__ import annotations

import argparse
import csv
import glob
import json
import os
import statistics as st
import subprocess
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(HERE, "tool", "target", "release", "promptdbg")
CORPUS = os.path.join(HERE, "corpus", "synthetic")


def truth_ids(stem: str) -> set[str] | None:
    """Ground-truth node ids from the deterministic annotation parser."""
    path = os.path.join(CORPUS, f"{stem}.rtpl")
    if not os.path.exists(path):
        path = os.path.join(CORPUS, f"{stem}")
        if not os.path.exists(path):
            return None
    proc = subprocess.run(
        [BIN if os.path.exists(BIN) else "promptdbg", "analyze", path,
         "--engine", "annotation", "--no-lint", "--format", "json"],
        capture_output=True, text=True, timeout=120,
    )
    if proc.returncode != 0:
        return None
    try:
        d = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return None
    return {n["id"] for n in d["ir"]["nodes"]}


def load_ir(path: str):
    try:
        j = json.load(open(path))
    except Exception:
        return None
    nodes = j.get("nodes") or (j.get("ir") or {}).get("nodes") or []
    allids, explicit = set(), set()
    for n in nodes:
        nid = n.get("id")
        if not nid:
            continue
        allids.add(nid)
        if (n.get("meta") or {}).get("inferred") is False:
            explicit.add(nid)
    return allids, explicit


def prf(pred: set, gold: set):
    if not pred and not gold:
        return 1.0, 1.0, 1.0
    tp = len(pred & gold)
    p = tp / len(pred) if pred else 0.0
    r = tp / len(gold) if gold else 0.0
    f = 2 * p * r / (p + r) if (p + r) else 0.0
    return p, r, f


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ir-dir", default=os.path.join(HERE, "results", "irs"))
    ap.add_argument("--out", default=os.path.join(HERE, "results", "ir_correctness.csv"))
    args = ap.parse_args()

    cache: dict[str, set[str] | None] = {}
    rows = []
    for path in sorted(glob.glob(os.path.join(args.ir_dir, "*", "*.json"))):
        model = os.path.basename(os.path.dirname(path)).replace("_", ":", 1)
        stem = os.path.basename(path).rsplit("_r", 1)[0]
        if stem not in cache:
            cache[stem] = truth_ids(stem)
        gold = cache[stem]
        if not gold:
            continue
        loaded = load_ir(path)
        if not loaded:
            continue
        allids, explicit = loaded
        p_all, r_all, f_all = prf(allids, gold)
        p_exp, _, _ = prf(explicit, gold)
        rows.append({
            "model": model, "prompt": stem,
            "run": os.path.basename(path).rsplit("_r", 1)[1].replace(".json", ""),
            "n_gold": len(gold), "n_inferred": len(allids), "n_explicit": len(explicit),
            "precision_all": round(p_all, 4), "recall": round(r_all, 4),
            "f1": round(f_all, 4), "precision_explicit": round(p_exp, 4),
        })

    if not rows:
        raise SystemExit("no comparable IRs found")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    by = defaultdict(list)
    for r in rows:
        by[r["model"]].append(r)

    hdr = f"{'model':24}{'runs':>6}{'recall':>9}{'prec(all)':>11}{'prec(expl)':>12}{'F1':>8}"
    print(hdr)
    print("-" * len(hdr))
    for m in sorted(by, key=lambda m: -st.mean(x["recall"] for x in by[m])):
        rs = by[m]
        print(f"{m:24}{len(rs):>6}"
              f"{st.mean(x['recall'] for x in rs):>9.2f}"
              f"{st.mean(x['precision_all'] for x in rs):>11.2f}"
              f"{st.mean(x['precision_explicit'] for x in rs):>12.2f}"
              f"{st.mean(x['f1'] for x in rs):>8.2f}")
    print("-" * len(hdr))
    print(f"{'all':24}{len(rows):>6}"
          f"{st.mean(x['recall'] for x in rows):>9.2f}"
          f"{st.mean(x['precision_all'] for x in rows):>11.2f}"
          f"{st.mean(x['precision_explicit'] for x in rows):>12.2f}"
          f"{st.mean(x['f1'] for x in rows):>8.2f}")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
