#!/usr/bin/env python3
"""Generate the paper's figures (PDF, vector) from the results CSVs.

Uses matplotlib only (no pandas). Run after the experiment scripts have
produced results/*.csv:

  python scripts/plots.py --out paper1-neurips/figures

The output directory is an argument because the same figures are consumed by
several submission directories; defaulting it to one of them silently leaves
the others holding stale PDFs.
"""
from __future__ import annotations

import argparse
import csv
import statistics as st
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

HERE = Path(__file__).resolve().parent.parent
RES = HERE / "results"
FIG = HERE / "paper" / "figures"  # overridden by --out in main()
plt.rcParams.update({"font.size": 9, "figure.dpi": 150})


def rows(name):
    p = RES / name
    return list(csv.DictReader(p.open())) if p.exists() else []


def fig_rq2():
    r = rows("rq2_baseline.csv")
    if not r:
        return
    r = sorted(r, key=lambda x: float(x["gain"]), reverse=True)
    single = [float(x["score_single"]) for x in r]
    gen = [float(x["score_generated"]) for x in r]
    idx = range(len(r))
    fig, ax = plt.subplots(figsize=(3.3, 2.1))
    ax.bar([i - 0.2 for i in idx], single, width=0.4, label="single input", color="#bbbbbb")
    ax.bar([i + 0.2 for i in idx], gen, width=0.4, label="coverage-directed", color="#4477aa")
    ax.set_xlabel("prompt (sorted by gain)")
    ax.set_ylabel("mutation score")
    ax.set_xticks([])
    ax.legend(frameon=False, fontsize=7, loc="lower left")
    fig.tight_layout()
    fig.savefig(FIG / "rq2_mutation.pdf")
    plt.close(fig)


def fig_rq4():
    # The merged per-cell table is the authoritative source: it applies the
    # paid-tier-wins precedence across the tier CSVs and so covers all eight
    # model variants. The older per-tier CSVs each hold only a subset.
    r = (rows("ir_stability_merged_cells.csv")
         or rows("ir_stability_combined.csv")
         or rows("ir_stability.csv"))
    if not r:
        return
    # Order models by descending median Jaccard so the figure reads left to right
    # from most to least reproducible, matching the table in the paper.
    models = sorted(
        {x["model"] for x in r},
        key=lambda m: -st.median([float(x["mean_jaccard"]) for x in r if x["model"] == m]),
    )
    fig, ax = plt.subplots(figsize=(6.6, 2.4))
    data_j = [[float(x["mean_jaccard"]) for x in r if x["model"] == m] for m in models]
    data_i = [[float(x["id_stability"]) for x in r if x["model"] == m] for m in models]
    pos = range(len(models))
    bpj = ax.boxplot(data_j, positions=[p - 0.18 for p in pos], widths=0.3,
                     patch_artist=True, showfliers=False)
    bpi = ax.boxplot(data_i, positions=[p + 0.18 for p in pos], widths=0.3,
                     patch_artist=True, showfliers=False)
    for b in bpj["boxes"]:
        b.set_facecolor("#4477aa")
    for b in bpi["boxes"]:
        b.set_facecolor("#ccbb44")
    ax.set_xticks(list(pos))
    ax.set_xticklabels(models, fontsize=7, rotation=20, ha="right")
    ax.set_ylabel("stability across runs")
    ax.set_ylim(-0.05, 1.05)
    ax.legend([bpj["boxes"][0], bpi["boxes"][0]], ["Jaccard", "ID-stability"],
              frameon=False, fontsize=7, loc="lower left")
    fig.tight_layout()
    fig.savefig(FIG / "rq4_stability.pdf")
    plt.close(fig)


def fig_operator_visibility():
    """Per-operator kills under the structural oracle versus the output oracle.

    The gap between the two bars is the share of a mutation score that no model
    could respond to, because the mutated annotation is stripped before
    rendering. An operator whose output bar is zero produces only mutants that
    are equivalent with respect to model behaviour.
    """
    r = rows("operator_visibility.csv")
    if not r:
        return
    r = sorted(r, key=lambda x: -int(x["killed_output"]))
    ops = [x["operator"] for x in r]
    trace = [int(x["killed_trace"]) for x in r]
    out = [int(x["killed_output"]) for x in r]
    idx = range(len(r))
    fig, ax = plt.subplots(figsize=(6.4, 2.5))
    ax.bar([i - 0.2 for i in idx], trace, width=0.4,
           label="structural (trace) oracle", color="#4477aa")
    ax.bar([i + 0.2 for i in idx], out, width=0.4,
           label="output oracle (model-visible)", color="#ccbb44")
    ax.set_xticks(list(idx))
    ax.set_xticklabels(ops, fontsize=7, rotation=15, ha="right")
    ax.set_ylabel("mutants killed")
    ax.legend(frameon=False, fontsize=7)
    for i, (t, o) in enumerate(zip(trace, out)):
        if t > o:
            ax.text(i, t + 6, f"-{t - o}", ha="center", fontsize=6, color="#aa3333")
    fig.tight_layout()
    fig.savefig(FIG / "operator_visibility.pdf")
    plt.close(fig)


def fig_coverage():
    r = rows("bench.csv")
    if not r:
        return
    cols = [("rule_cov_structural", "rule"), ("condition_cov", "cond"),
            ("branch_cov", "branch"), ("rule_pair_cov", "pair")]
    data = [[float(x[c]) for x in r if x.get(c)] for c, _ in cols]
    fig, ax = plt.subplots(figsize=(3.3, 2.1))
    bp = ax.boxplot(data, patch_artist=True, showfliers=True)
    for b in bp["boxes"]:
        b.set_facecolor("#44aa77")
    ax.set_xticklabels([lbl for _, lbl in cols])
    ax.set_ylabel("coverage")
    ax.set_ylim(-0.05, 1.05)
    fig.tight_layout()
    fig.savefig(FIG / "coverage.pdf")
    plt.close(fig)


def fig_rq3():
    r = rows("rq3_detection.csv")
    if not r:
        return
    # Prefer multi-run rate columns when present; fall back to single-run 0/1.
    key_w = "rate_with_ir" if "rate_with_ir" in r[0] else "detected_with_ir"
    key_o = "rate_without_ir" if "rate_without_ir" in r[0] else "detected_without_ir"
    xs_w = [float(x[key_w]) for x in r]
    xs_o = [float(x[key_o]) for x in r]
    with_ir, without_ir = st.mean(xs_w), st.mean(xs_o)
    n = len(r)
    ci95 = lambda xs: (1.96 * st.pstdev(xs) / (n ** 0.5)) if n >= 2 else 0.0
    err_w, err_o = ci95(xs_w), ci95(xs_o)
    fig, ax = plt.subplots(figsize=(3.3, 2.1))
    means = [1.0, with_ir, without_ir]
    errs = [0.0, err_w, err_o]
    bars = ax.bar(["static\nanalyser", "LLM\n+IR", "LLM\n-IR"],
                  means, yerr=errs, capsize=3,
                  color=["#44aa77", "#4477aa", "#aabbcc"])
    runs = r[0].get("runs_with_ir", "1")
    ax.set_ylabel(f"detection rate (n={n} defects, R={runs} runs)")
    ax.set_ylim(0, 1.05)
    for b, v in zip(bars, means):
        ax.text(b.get_x() + b.get_width() / 2, v + 0.04, f"{v:.2f}",
                ha="center", fontsize=8)
    fig.tight_layout()
    fig.savefig(FIG / "rq3_recall.pdf")
    plt.close(fig)


def main() -> None:
    global FIG
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--out",
        default=str(HERE / "paper" / "figures"),
        help="directory to write the figure PDFs into",
    )
    args = ap.parse_args()
    FIG = Path(args.out)
    FIG.mkdir(parents=True, exist_ok=True)

    fig_rq2()
    fig_rq4()
    fig_coverage()
    fig_rq3()
    fig_operator_visibility()
    print("wrote figures to", FIG)
    for f in sorted(FIG.glob("*.pdf")):
        print("  ", f.name)


if __name__ == "__main__":
    main()
