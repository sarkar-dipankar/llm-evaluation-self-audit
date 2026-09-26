#!/usr/bin/env python3
"""Merge the per-tier IR-stability CSVs into the authoritative per-model table.

The stability study was run in several passes: an initial free-tier pass, a
scaled free-tier pass, and three paid-tier passes (B, C, D). A given
(model, prompt) pair can therefore appear in more than one CSV. The paid-tier
runs used a larger wall-clock budget per inference (300s vs 180s), so they are
the higher-quality measurement and win wherever a pair appears in both.

Precedence, highest first:

    ir_stability_paid_tierD.csv
    ir_stability_paid_tierC.csv
    ir_stability_paid_tierB.csv
    ir_stability_scaled.csv
    ir_stability_combined.csv
    ir_stability_qwen.csv
    ir_stability_ministral.csv
    ir_stability_minimax.csv
    ir_stability_xmodel.csv
    ir_stability.csv

The real-corpus runs (ir_stability_real*.csv) are deliberately excluded: they
measure a different corpus (GitHub SKILL.md files) and are reported separately
as RQ3. Empty CSVs (deepseek, glm, ministral_v2) contribute nothing.

A prompt-model cell counts as "perfect" when both mean_jaccard and id_stability
are 1.0, i.e. every pair of runs produced an identical node set with identical
ids.

Usage:
    python3 scripts/merge_ir_stability.py [--out results/ir_stability_merged.csv]
"""
from __future__ import annotations

import argparse
import csv
import os
import statistics as st
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RESULTS = os.path.join(HERE, "results")

# Highest precedence first. Paid tiers outrank free tiers.
PRECEDENCE = [
    "ir_stability_paid_tierD.csv",
    "ir_stability_paid_tierC.csv",
    "ir_stability_paid_tierB.csv",
    "ir_stability_scaled.csv",
    "ir_stability_combined.csv",
    "ir_stability_qwen.csv",
    "ir_stability_ministral.csv",
    "ir_stability_minimax.csv",
    "ir_stability_xmodel.csv",
    "ir_stability.csv",
]

# Reported separately as RQ3 (real GitHub SKILL.md corpus), not merged in here.
REAL_CORPUS = "ir_stability_real_v2.csv"

PERFECT_EPS = 1e-9


def _read(name: str) -> list[dict]:
    path = os.path.join(RESULTS, name)
    if not os.path.exists(path):
        return []
    with open(path, newline="") as fh:
        return list(csv.DictReader(fh))


def _is_perfect(row: dict) -> bool:
    return (
        float(row["mean_jaccard"]) >= 1.0 - PERFECT_EPS
        and float(row["id_stability"]) >= 1.0 - PERFECT_EPS
    )


def merge_cells() -> dict[tuple[str, str], tuple[str, dict]]:
    """Return {(model, prompt): (source_file, row)} under the precedence rule."""
    best: dict[tuple[str, str], tuple[str, dict]] = {}
    for name in PRECEDENCE:
        for row in _read(name):
            key = (row["model"], row["prompt"])
            if key not in best:  # first file in precedence order wins
                best[key] = (name, row)
    return best


def summarise(cells: dict[tuple[str, str], tuple[str, dict]]) -> list[dict]:
    by_model: dict[str, list[dict]] = defaultdict(list)
    for (model, _prompt), (_src, row) in cells.items():
        by_model[model].append(row)

    out = []
    for model, rows in by_model.items():
        out.append(
            {
                "model": model,
                "n_prompts": len(rows),
                "mean_jaccard": round(st.mean(float(r["mean_jaccard"]) for r in rows), 4),
                "id_stability": round(st.mean(float(r["id_stability"]) for r in rows), 4),
                "perfect": sum(1 for r in rows if _is_perfect(r)),
                "mean_nodes": round(st.mean(float(r["mean_nodes"]) for r in rows), 3),
                "mean_confidence": round(
                    st.mean(float(r["mean_confidence"]) for r in rows), 4
                ),
            }
        )
    out.sort(key=lambda d: -d["mean_jaccard"])
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--out",
        default=os.path.join(RESULTS, "ir_stability_merged.csv"),
        help="where to write the merged per-model table",
    )
    ap.add_argument(
        "--cells-out",
        default=os.path.join(RESULTS, "ir_stability_merged_cells.csv"),
        help="where to write the per-cell provenance table",
    )
    args = ap.parse_args()

    cells = merge_cells()
    table = summarise(cells)

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(table[0].keys()))
        w.writeheader()
        w.writerows(table)

    with open(args.cells_out, "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["model", "prompt", "source_file", "mean_jaccard", "id_stability", "perfect"])
        for (model, prompt), (src, row) in sorted(cells.items()):
            w.writerow(
                [model, prompt, src, row["mean_jaccard"], row["id_stability"], int(_is_perfect(row))]
            )

    total_n = sum(r["n_prompts"] for r in table)
    total_perfect = sum(r["perfect"] for r in table)

    hdr = f"{'model':24}{'n':>4}{'Jaccard':>9}{'ID-stab':>9}{'perfect':>9}"
    print(hdr)
    print("-" * len(hdr))
    for r in table:
        perfect = f"{r['perfect']}/{r['n_prompts']}"
        print(
            f"{r['model']:24}{r['n_prompts']:>4}{r['mean_jaccard']:>9.2f}"
            f"{r['id_stability']:>9.2f}{perfect:>9}"
        )
    print("-" * len(hdr))
    print(f"{'total':24}{total_n:>4}{'':>9}{'':>9}{f'{total_perfect}/{total_n}':>9}")
    print(
        f"\nperfectly stable cells: {total_perfect}/{total_n} "
        f"({100.0 * total_perfect / total_n:.1f}%)"
    )
    print(
        f"Jaccard range across models: "
        f"[{min(r['mean_jaccard'] for r in table):.2f}, {max(r['mean_jaccard'] for r in table):.2f}]"
    )
    print(
        f"ID-stability range across models: "
        f"[{min(r['id_stability'] for r in table):.2f}, {max(r['id_stability'] for r in table):.2f}]"
    )

    real = _read(REAL_CORPUS)
    if real:
        print(
            f"\nreal corpus ({REAL_CORPUS}): n={len(real)}, "
            f"J={st.mean(float(r['mean_jaccard']) for r in real):.3f}, "
            f"ID={st.mean(float(r['id_stability']) for r in real):.3f}, "
            f"perfect={sum(1 for r in real if _is_perfect(r))}/{len(real)}"
        )

    print(f"\nwrote {args.out}")
    print(f"wrote {args.cells_out}")


if __name__ == "__main__":
    main()
