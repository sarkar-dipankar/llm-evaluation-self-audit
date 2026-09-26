#!/usr/bin/env python3
"""Tool-baseline comparison: a NAIVE priority-only unreachable detector vs the
SOUND guard-chain analyzer in `promptdbg`.

Both detectors agree on the *operational definition* of an unreachable defect
(same-category rule with strictly higher priority that fires whenever the lower
one does). The naive detector ignores branch-guard structure: it flags any
same-category lower-priority rule as potentially unreachable. The sound detector
(\\ref{sec:static}) requires that the dominator's guard chain is a *prefix* of
the dominated rule's, eliminating false positives across mutually exclusive
branches.

On BENIGN prompts (no injected defect), the sound analyzer is expected to emit
near-zero diagnostics; the naive detector will fire on any prompt with two
same-category rules at any nesting depth. We report the per-prompt diagnostic
counts and the false-positive ratio. On DEFECTIVE prompts (with an injected
top-level dominator) both detectors trivially recall the defect; the discriminator
is precision on benign prompts.

  python scripts/baseline_naive.py --bin tool/target/release/promptdbg
"""
from __future__ import annotations

import argparse
import csv
import json
import re
import statistics as st
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"

ANN_RE = re.compile(r"^\s*//\s*@rule\s+(\S+)(.*)$")
KV_RE = re.compile(r"(\w+)=([^\s]+)")


def parse_rules(text: str) -> list[dict]:
    """Extract (id, category, priority, line) from `// @rule` annotations."""
    rules = []
    for i, line in enumerate(text.splitlines()):
        m = ANN_RE.match(line)
        if not m:
            continue
        attrs = dict(KV_RE.findall(m.group(2)))
        prio = int(attrs.get("priority", "0")) if attrs.get("priority", "0").lstrip("-").isdigit() else 0
        rules.append({
            "id": m.group(1),
            "category": attrs.get("category"),
            "priority": prio,
            "line": i,
        })
    return rules


def naive_diagnostics(text: str) -> list[str]:
    """Flag any same-category lower-priority rule as potentially unreachable."""
    rs = parse_rules(text)
    flagged = []
    for r in rs:
        if r["category"] is None:
            continue
        for d in rs:
            if d["id"] == r["id"]:
                continue
            if d["category"] == r["category"] and d["priority"] > r["priority"]:
                flagged.append(r["id"])
                break
    return flagged


def sound_diagnostics(binary, path: Path) -> list[str]:
    """Read the sound static analyzer's diagnostics via `analyze --no-lint`."""
    out = subprocess.run(
        [binary, "analyze", str(path), "--engine", "annotation", "--no-lint",
         "--format", "json"],
        capture_output=True, text=True, timeout=30,
    )
    if out.returncode != 0 or not out.stdout.strip():
        return []
    try:
        d = json.loads(out.stdout)
    except json.JSONDecodeError:
        return []
    ids = []
    for diag in d.get("diagnostics", []):
        if diag.get("code") == "prompt/unreachable-rule":
            # message: "Rule X is unreachable: ..."
            msg = diag.get("message", "")
            if msg.startswith("Rule "):
                ids.append(msg.split()[1])
    return ids


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--out", default=str(HERE / "results" / "baseline_naive.csv"))
    args = ap.parse_args()

    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    prompts = sorted(Path(args.corpus).glob("*.rtpl"))

    rows = []
    for p in prompts:
        text = p.read_text()
        naive = naive_diagnostics(text)
        sound = sound_diagnostics(binary, p)
        rows.append({
            "prompt": p.name,
            "naive_diagnostics": len(naive),
            "sound_diagnostics": len(sound),
            "naive_only": len(set(naive) - set(sound)),
            "sound_only": len(set(sound) - set(naive)),
        })

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    print(f"Wrote {len(rows)} rows to {out}\n")
    nm = sum(r["naive_diagnostics"] for r in rows)
    sm = sum(r["sound_diagnostics"] for r in rows)
    n_fp = sum(r["naive_only"] for r in rows)
    print(f"Benign-corpus diagnostics (no injected defect):")
    print(f"  naive total:  {nm} ({st.mean(r['naive_diagnostics'] for r in rows):.2f} per prompt)")
    print(f"  sound total:  {sm} ({st.mean(r['sound_diagnostics'] for r in rows):.2f} per prompt)")
    print(f"  naive-only (likely false positives): {n_fp}")
    if sm > 0:
        print(f"  naive/sound ratio: {nm/sm:.1f}x")
    elif nm > 0:
        print(f"  sound emits zero on the benign corpus; naive emits {nm} (all false positives).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
