#!/usr/bin/env python3
"""RQ2 measured under both oracles.

`rq2_baseline.py` reports the suite-adequacy gain under the structural (trace)
oracle only. Since this work's own recommendation is that prompt-mutation results
be reported under a text-level oracle as well, reporting one of them here would be
inconsistent. This script runs the same single-input versus coverage-directed
comparison twice, once per oracle, so both numbers appear together.

Usage:
    python3 scripts/rq2_both_oracles.py [--out results/rq2_both_oracles.csv]
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import subprocess
import tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_BIN = os.path.join(HERE, "tool", "target", "release", "promptdbg")
CORPUS = os.path.join(HERE, "corpus", "synthetic")
CONTEXTS = os.path.join(HERE, "corpus", "contexts")

SCORE = re.compile(r"Mutation score:\s+(\d+)/(\d+)")


def score(binary: str, prompt: str, contexts: str, oracle: str):
    proc = subprocess.run(
        [binary, "mutate", prompt, "--contexts", contexts, "--oracle", oracle],
        capture_output=True, text=True, timeout=300,
    )
    if proc.returncode != 0:
        return None
    m = SCORE.search(proc.stdout)
    if not m:
        return None
    killed, total = int(m.group(1)), int(m.group(2))
    return (killed / total if total else 0.0), killed, total


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bin", default=DEFAULT_BIN)
    ap.add_argument("--out", default=os.path.join(HERE, "results", "rq2_both_oracles.csv"))
    args = ap.parse_args()
    binary = args.bin if os.path.exists(args.bin) else "promptdbg"

    tmp = tempfile.mkdtemp(prefix="rq2both_")
    rows = []
    for name in sorted(os.listdir(CORPUS)):
        if not name.endswith(".prompt.rtpl"):
            continue
        stem = name[: -len(".prompt.rtpl")]
        full = os.path.join(CONTEXTS, f"{stem}.contexts.json")
        if not os.path.exists(full):
            continue
        prompt = os.path.join(CORPUS, name)

        try:
            contexts = json.load(open(full))
        except Exception:
            continue
        if not contexts:
            continue

        # The single-input baseline is the first generated context, matching
        # rq2_baseline.py so the two scripts are comparable.
        single = os.path.join(tmp, f"{stem}.single.json")
        with open(single, "w") as fh:
            json.dump(contexts[:1], fh)

        row = {"prompt": name, "n_contexts": len(contexts)}
        ok = True
        for oracle in ("trace", "output"):
            s1 = score(binary, prompt, single, oracle)
            sg = score(binary, prompt, full, oracle)
            if s1 is None or sg is None:
                ok = False
                break
            row[f"single_{oracle}"] = round(s1[0], 4)
            row[f"generated_{oracle}"] = round(sg[0], 4)
            row[f"gain_{oracle}"] = round(sg[0] - s1[0], 4)
        if ok:
            rows.append(row)

    if not rows:
        raise SystemExit("no prompts scored; is the binary built?")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    import statistics as st
    print(f"prompts: {len(rows)}")
    print(f"{'oracle':10}{'single':>10}{'directed':>10}{'gain':>10}{'improved':>11}{'max gain':>10}")
    for oracle in ("trace", "output"):
        s = st.mean(r[f"single_{oracle}"] for r in rows)
        g = st.mean(r[f"generated_{oracle}"] for r in rows)
        gains = [r[f"gain_{oracle}"] for r in rows]
        imp = sum(1 for x in gains if x > 1e-9)
        print(f"{oracle:10}{s:>10.4f}{g:>10.4f}{st.mean(gains):>10.4f}"
              f"{f'{imp}/{len(rows)}':>11}{max(gains):>10.4f}")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
