#!/usr/bin/env python3
"""Drift decomposition for Paper 2.

Given a directory of per-run raw IR JSONs (produced by `ir_stability.py
--save-irs`), classify each pairwise disagreement between runs into one of:

  * id-label drift   : same node count and same kind distribution, different ids
  * count drift      : node counts differ between runs
  * hierarchy drift  : counts and ids match, but per-node metadata (range / category /
                       priority) differs
  * (other)          : disagreements that fall outside the above three.

Per-prompt and per-model summaries are written to results/drift_decompose.csv.

Layout expected:
  irs/<model_slug>/<prompt_stem>_r<run>.json
where <model_slug> is the model name with `:` -> `_`.

  python scripts/drift_decompose.py --ir-dir results/irs
"""
from __future__ import annotations

import argparse
import csv
import itertools
import json
import statistics as st
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent


def load_run(path: Path) -> list[dict] | None:
    try:
        data = json.loads(path.read_text())
    except Exception:
        return None
    return data.get("nodes", [])


def kind_dist(nodes: list[dict]) -> Counter:
    return Counter(n.get("kind", "?") for n in nodes)


def ids_of(nodes: list[dict]) -> set:
    return {n.get("id") for n in nodes if n.get("id") is not None}


def classify_pair(a: list[dict], b: list[dict]) -> str:
    if len(a) != len(b):
        return "count"
    ka, kb = kind_dist(a), kind_dist(b)
    if ka != kb:
        return "count"  # different mix of node types is a count-like change
    ia, ib = ids_of(a), ids_of(b)
    if ia != ib:
        return "id"
    # same ids: compare per-id metadata for hierarchy/range/category changes
    by_id = {n["id"]: n for n in a}
    for n in b:
        m = by_id.get(n["id"])
        if not m:
            return "hierarchy"
        # compare meta + range
        if m.get("range") != n.get("range") or m.get("meta") != n.get("meta"):
            return "hierarchy"
    return "other"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ir-dir", default=str(HERE / "results" / "irs"))
    ap.add_argument("--out", default=str(HERE / "results" / "drift_decompose.csv"))
    args = ap.parse_args()

    root = Path(args.ir_dir)
    if not root.exists():
        print(f"No IR dir at {root}. Run ir_stability with --save-irs first.")
        return 1

    rows = []
    for model_dir in sorted(p for p in root.iterdir() if p.is_dir()):
        model = model_dir.name.replace("_", ":", 1)
        by_prompt: dict[str, list[Path]] = defaultdict(list)
        for f in sorted(model_dir.glob("*_r*.json")):
            stem = f.stem.rsplit("_r", 1)[0]
            by_prompt[stem].append(f)
        for stem, files in by_prompt.items():
            runs = [load_run(f) for f in files]
            runs = [r for r in runs if r is not None]
            if len(runs) < 2:
                continue
            pairs = list(itertools.combinations(runs, 2))
            tally = Counter(classify_pair(a, b) for a, b in pairs)
            total = sum(tally.values())
            disagreements = total - tally.get("other", 0)
            rows.append({
                "model": model, "prompt": stem,
                "runs": len(runs), "pairs": total,
                "id_drift": tally.get("id", 0),
                "count_drift": tally.get("count", 0),
                "hierarchy_drift": tally.get("hierarchy", 0),
                "agree": tally.get("other", 0),
                "frac_id": round(tally.get("id", 0) / max(1, disagreements), 3),
                "frac_count": round(tally.get("count", 0) / max(1, disagreements), 3),
                "frac_hier": round(tally.get("hierarchy", 0) / max(1, disagreements), 3),
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
            agg[r["model"]].append(r)
        print(f"{'model':<24}{'n':>4}{'%id':>8}{'%cnt':>8}{'%hier':>8}")
        for m, rs in agg.items():
            mid = st.mean(r["frac_id"] for r in rs)
            mc = st.mean(r["frac_count"] for r in rs)
            mh = st.mean(r["frac_hier"] for r in rs)
            print(f"{m:<24}{len(rs):>4}{100*mid:>7.1f}%{100*mc:>7.1f}%{100*mh:>7.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
