#!/usr/bin/env python3
"""RQ3: does the LLM linter detect injected unreachable-rule defects, and does the
prompt IR help it?

Method (deterministic ground truth + LLM evaluation):
  1. For each corpus prompt, inject a top-level high-priority rule in an existing
     category. This makes the existing same-category rules unreachable.
  2. Ground truth = the deterministic `unreachable_rules` static lint
     (`analyze --no-lint`, code prompt/unreachable-rule). Skip prompts where the
     injection produced no shadowed rule.
  3. Run the LLM linter on the defective prompt WITH the IR
     (`analyze --no-static-lint`) and WITHOUT it (`--ablate-ir`); a defect is
     "detected" if any LLM diagnostic indicates a conflict/unreachable/shadowing.
  4. Report detection recall with vs without IR.

Note: annotation comments (with priority=) remain in the raw prompt text, so the
"without IR" condition still has the metadata in text; this measures the marginal
value of the *structured* IR. Usage:

  OLLAMA_API_KEY=... python scripts/rq3_detection.py --model gemma3:12b
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"

DETECT_KEYWORDS = ("unreachable", "conflict", "shadow", "never", "overrid", "precede", "priority")


def run_json(binary, args, env=None, timeout=120):
    out = subprocess.run([binary, *args], capture_output=True, text=True, env=env, timeout=timeout)
    if out.returncode != 0 or not out.stdout.strip():
        return None
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return None


def categories(binary, prompt: Path):
    """Return {category: [rule_ids]} from the deterministic IR."""
    d = run_json(binary, ["analyze", str(prompt), "--engine", "annotation", "--no-lint",
                          "--format", "json"])
    cats: dict[str, list[str]] = {}
    if d:
        for n in d["ir"]["nodes"]:
            if n["kind"] == "Rule":
                c = n.get("meta", {}).get("category")
                if c:
                    cats.setdefault(c, []).append(n["id"])
    return cats


def static_unreachable(binary, prompt: Path) -> list[str]:
    """Ground-truth unreachable rule ids (static lint)."""
    d = run_json(binary, ["analyze", str(prompt), "--engine", "annotation", "--no-lint",
                          "--format", "json"])
    ids = []
    if d:
        for diag in d.get("diagnostics", []):
            if diag.get("code") == "prompt/unreachable-rule":
                # message: "Rule X is unreachable: ..."
                msg = diag.get("message", "")
                if msg.startswith("Rule "):
                    ids.append(msg.split()[1])
    return ids


def llm_detects(binary, prompt: Path, env, with_ir: bool, timeout: int) -> bool | None:
    args = ["analyze", str(prompt), "--engine", "annotation", "--no-static-lint", "--format", "json"]
    if not with_ir:
        args.append("--ablate-ir")
    d = run_json(binary, args, env=env, timeout=timeout)
    if d is None:
        return None
    for diag in d.get("diagnostics", []):
        blob = f"{diag.get('code', '')} {diag.get('message', '')}".lower()
        if any(k in blob for k in DETECT_KEYWORDS):
            return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--model", default="gemma3:12b")
    ap.add_argument("--timeout", type=int, default=110)
    ap.add_argument("--repeats", type=int, default=1,
                    help="repeat the LLM-detection call this many times per condition")
    ap.add_argument("--out", default=str(HERE / "results" / "rq3_detection.csv"))
    ap.add_argument("--max", type=int, default=0,
                    help="cap prompts processed (0 = no cap); bounds LLM call budget")
    args = ap.parse_args()

    if not os.environ.get("OLLAMA_API_KEY") and "OPENAI_API_KEY" not in os.environ:
        sys.exit("Set OLLAMA_API_KEY (LLM linter required for RQ3).")
    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    env = dict(os.environ, OLLAMA_MODEL=args.model)
    prompts = sorted(Path(args.corpus).glob("*.rtpl"))
    if args.max > 0:
        prompts = prompts[:args.max]

    rows = []
    tmp = Path(tempfile.mkdtemp(prefix="rq3_"))
    for p in prompts:
        cats = categories(binary, p)
        if not cats:
            continue
        # Inject a dominating rule in the most common category.
        cat = max(cats, key=lambda c: len(cats[c]))
        text = p.read_text()
        defect = f"// @rule RULE:INJECTED_DOMINATOR category={cat} priority=999\n" + text
        dpath = tmp / p.name
        dpath.write_text(defect)

        truth = static_unreachable(binary, dpath)
        if not truth:
            print(f"  {p.name}: injection shadowed nothing, skipping", file=sys.stderr)
            continue

        runs_with: list[int] = []
        runs_without: list[int] = []
        for _ in range(args.repeats):
            w = llm_detects(binary, dpath, env, True, args.timeout)
            o = llm_detects(binary, dpath, env, False, args.timeout)
            if w is not None: runs_with.append(int(w))
            if o is not None: runs_without.append(int(o))
        if not runs_with or not runs_without:
            print(f"  {p.name}: LLM lint failed for all runs, skipping", file=sys.stderr)
            continue
        rate_w = sum(runs_with) / len(runs_with)
        rate_o = sum(runs_without) / len(runs_without)
        rows.append({
            "prompt": p.name,
            "injected_category": cat,
            "ground_truth_unreachable": len(truth),
            "runs_with_ir": len(runs_with),
            "runs_without_ir": len(runs_without),
            "rate_with_ir": round(rate_w, 3),
            "rate_without_ir": round(rate_o, 3),
            # Kept for backward compatibility with the existing 1-run pipeline.
            "detected_with_ir": int(rate_w >= 0.5),
            "detected_without_ir": int(rate_o >= 0.5),
        })
        print(f"  {p.name}: truth={len(truth)} rate_with={rate_w:.2f}/{len(runs_with)} "
              f"rate_without={rate_o:.2f}/{len(runs_without)}", file=sys.stderr)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    fields = (list(rows[0].keys()) if rows else
              ["prompt", "injected_category", "ground_truth_unreachable",
               "runs_with_ir", "runs_without_ir", "rate_with_ir", "rate_without_ir",
               "detected_with_ir", "detected_without_ir"])
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    print(f"\nWrote {len(rows)} labeled defects to {out}")
    if rows:
        # Per-prompt detection rates across the N runs; aggregate over prompts.
        import statistics as _st
        rates_w = [r["rate_with_ir"] for r in rows]
        rates_o = [r["rate_without_ir"] for r in rows]
        n = len(rows)
        def ci95(xs):
            if n < 2: return 0.0
            return 1.96 * _st.pstdev(xs) / (n ** 0.5)
        mean_w, mean_o = _st.mean(rates_w), _st.mean(rates_o)
        diffs = [w - o for w, o in zip(rates_w, rates_o)]
        print(f"LLM-linter detection rate over {args.repeats} runs/condition, {n} defects:")
        print(f"  with IR:    {mean_w:.3f} +/- {ci95(rates_w):.3f}  (95% CI)")
        print(f"  without IR: {mean_o:.3f} +/- {ci95(rates_o):.3f}")
        print(f"  paired mean diff (with - without): {_st.mean(diffs):+.3f} +/- {ci95(diffs):.3f}")
        print(f"(deterministic static analyzer recall = 1.00 by construction;")
        print(f" gap = 1.00 - mean_with = {1.0 - mean_w:.3f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
