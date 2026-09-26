#!/usr/bin/env python3
"""Per-operator kill rates under the trace oracle versus the output oracle.

Annotations in the prompt format are comments and are stripped when the prompt
is rendered. An operator that edits only an annotation therefore produces a
mutant whose rendered text is identical to the original, and no output-based
oracle can ever kill it. Such mutants are equivalent with respect to model
behaviour, however much structural bookkeeping they disturb.

This script quantifies that directly: for every prompt in the corpus it runs
mutation testing twice, once with `--oracle trace` and once with
`--oracle output`, and reports kills per operator under each. The gap between
the two columns is the share of the mutation score that reflects metadata
rather than anything the model could read.

Usage:
    python3 scripts/operator_visibility.py [--out results/operator_visibility.csv]
"""
from __future__ import annotations

import argparse
import csv
import os
import re
import subprocess
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_BIN = os.path.join(HERE, "tool", "target", "release", "promptdbg")
CORPUS = os.path.join(HERE, "corpus", "synthetic")
CONTEXTS = os.path.join(HERE, "corpus", "contexts")

# "  DropRule         5/5"
# The tool appends a "(N metadata-only, M body-carrying)" note to each row.
ROW = re.compile(r"^\s+(\w+)\s+(\d+)/(\d+)\s*(?:\((.*)\))?\s*$")
META = re.compile(r"(\d+)\s+metadata-only")


def run(binary: str, prompt: str, contexts: str, oracle: str) -> dict[str, tuple[int, int, int]]:
    """Return {operator: (killed, total, metadata_only)} for one prompt under one oracle.

    The tool tags a mutant `metadata-only` when the site it edits carries no
    body, so the mutation is confined to an annotation and cannot change the
    rendered text. Those are the degenerate sites of a body-carrying operator,
    and we count them because they bound how much of a body-carrying operator's
    score is still coming from metadata."""
    proc = subprocess.run(
        [binary, "mutate", prompt, "--contexts", contexts, "--oracle", oracle],
        capture_output=True,
        text=True,
        timeout=300,
    )
    if proc.returncode != 0:
        return {}
    out: dict[str, tuple[int, int]] = {}
    in_table = False
    for line in proc.stdout.splitlines():
        if line.startswith("By operator:"):
            in_table = True
            continue
        if in_table:
            m = ROW.match(line)
            if not m:
                if line.strip():  # table finished
                    break
                continue
            note = m.group(4) or ""
            mm = META.search(note)
            out[m.group(1)] = (int(m.group(2)), int(m.group(3)), int(mm.group(1)) if mm else 0)
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bin", default=DEFAULT_BIN)
    ap.add_argument("--out", default=os.path.join(HERE, "results", "operator_visibility.csv"))
    args = ap.parse_args()

    binary = args.bin if os.path.exists(args.bin) else "promptdbg"

    totals: dict[str, dict[str, int]] = defaultdict(
        lambda: {"total": 0, "trace": 0, "output": 0, "meta": 0})
    n_prompts = 0
    for name in sorted(os.listdir(CORPUS)):
        if not name.endswith(".prompt.rtpl"):
            continue
        stem = name[: -len(".prompt.rtpl")]
        ctx = os.path.join(CONTEXTS, f"{stem}.contexts.json")
        if not os.path.exists(ctx):
            ctx = os.path.join(CONTEXTS, "default.contexts.json")
        prompt = os.path.join(CORPUS, name)

        tr = run(binary, prompt, ctx, "trace")
        ou = run(binary, prompt, ctx, "output")
        if not tr:
            continue  # unrenderable prompt, already excluded from bench.csv
        n_prompts += 1
        for op, (killed, total, meta) in tr.items():
            totals[op]["total"] += total
            totals[op]["trace"] += killed
            totals[op]["meta"] += meta
        for op, (killed, _total, _meta) in ou.items():
            totals[op]["output"] += killed

    if not totals:
        sys.exit("no prompts produced mutation output; is the binary built?")

    rows = []
    for op in sorted(totals):
        d = totals[op]
        rows.append(
            {
                "operator": op,
                "mutants": d["total"],
                "killed_trace": d["trace"],
                "killed_output": d["output"],
                "rate_trace": round(d["trace"] / d["total"], 4) if d["total"] else 0.0,
                "rate_output": round(d["output"] / d["total"], 4) if d["total"] else 0.0,
                "metadata_only_sites": d["meta"],
                "model_visible": "yes" if d["output"] > 0 else "no",
            }
        )

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    hdr = (f"{'operator':18}{'mutants':>9}{'trace':>9}{'output':>9}"
           f"{'rate_tr':>9}{'rate_out':>10}{'meta-only':>11}  visible")
    print(f"prompts: {n_prompts}")
    print(hdr)
    print("-" * len(hdr))
    for r in rows:
        print(
            f"{r['operator']:18}{r['mutants']:>9}{r['killed_trace']:>9}{r['killed_output']:>9}"
            f"{r['rate_trace']:>9.2f}{r['rate_output']:>10.2f}"
            f"{r['metadata_only_sites']:>11}  {r['model_visible']}"
        )
    tot = sum(r["mutants"] for r in rows)
    ktr = sum(r["killed_trace"] for r in rows)
    kou = sum(r["killed_output"] for r in rows)
    print("-" * len(hdr))
    print(f"{'total':18}{tot:>9}{ktr:>9}{kou:>9}{ktr/tot:>9.2f}{kou/tot:>10.2f}")
    print(
        f"\nkills invisible to the model: {ktr - kou}/{ktr} "
        f"({100.0 * (ktr - kou) / ktr:.0f}% of trace-oracle kills)"
    )
    for r in rows:
        if r["model_visible"] == "yes" and r["metadata_only_sites"]:
            print(f"  {r['operator']}: {r['metadata_only_sites']}/{r['mutants']} sites are "
                  f"degenerate (bodiless), {100.0*r['metadata_only_sites']/r['mutants']:.0f}%")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
