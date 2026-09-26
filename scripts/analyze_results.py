#!/usr/bin/env python3
"""Summarize a bench.csv produced by `promptdbg bench`.

Prints mean/median coverage and mutation score across the corpus. Uses pandas if
available, otherwise falls back to the stdlib csv module.

Usage:
  python scripts/analyze_results.py results/bench.csv
"""
from __future__ import annotations

import statistics
import sys
from pathlib import Path

NUMERIC = [
    "rule_cov_structural", "rule_cov_trace", "condition_cov",
    "branch_cov", "rule_pair_cov", "mutation_score",
]


def main() -> int:
    path = Path(sys.argv[1] if len(sys.argv) > 1 else "results/bench.csv")
    if not path.exists():
        sys.exit(f"No such file: {path}")

    try:
        import pandas as pd  # noqa: WPS433

        df = pd.read_csv(path)
        print(f"Corpus: {len(df)} prompts\n")
        present = [c for c in NUMERIC if c in df.columns]
        summary = df[present].agg(["mean", "median", "min", "max"]).round(3)
        print(summary.to_string())
        return 0
    except ImportError:
        pass

    import csv

    rows = list(csv.DictReader(path.open()))
    print(f"Corpus: {len(rows)} prompts\n")
    for col in NUMERIC:
        vals = [float(r[col]) for r in rows if r.get(col) not in (None, "")]
        if vals:
            print(
                f"{col:<20} mean={statistics.mean(vals):.3f} "
                f"median={statistics.median(vals):.3f} "
                f"min={min(vals):.3f} max={max(vals):.3f}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
