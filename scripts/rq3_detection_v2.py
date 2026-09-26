#!/usr/bin/env python3
"""RQ3 (v2): can the LLM linter detect an *injected* unreachable-rule defect?

The first version of this experiment (`rq3_detection.py`) counted a detection
whenever any LLM diagnostic on the defective prompt contained one of a handful
of keywords, among them the word "priority". Two things were wrong with that:

  1. No before/after comparison. Most corpus prompts already carry unreachable
     rules before anything is injected, so the linter could score a "detection"
     by remarking on a pre-existing conflict it would have mentioned anyway.
  2. No attribution. A diagnostic never had to name the rule that the injection
     actually shadowed, so "detected" did not mean "found this defect".

This version fixes both and reports precision as well as recall:

  * Ground truth is the *delta*: rules the static analyser reports as
    unreachable after injection minus those it reported before. If the delta is
    empty the prompt is skipped, because the injection did not create a defect.
  * The linter is run on the original prompt and on the injected prompt in the
    same condition, and only diagnostics that are new in the injected run count.
  * A detection is STRICT when a new diagnostic names a rule in the delta, and
    LENIENT when a new diagnostic merely matches the old keyword list. Both are
    reported so the cost of each tightening is visible.
  * Every raw diagnostic is written to disk so the classification is auditable.

Usage:
  OLLAMA_API_KEY=... python3 scripts/rq3_detection_v2.py --repeats 3 --max 25
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"

# The original, deliberately permissive keyword list, kept so the lenient
# measure is exactly comparable to the v1 number.
DETECT_KEYWORDS = ("unreachable", "conflict", "shadow", "never", "overrid", "precede", "priority")

RULE_TOKEN = re.compile(r"\bRULE:[A-Za-z0-9_]+\b")


def run_json(binary, args, env=None, timeout=120):
    """Run the tool and parse its JSON, or return None if the call did not succeed.

    The tool logs provider errors (429 rate limits, timeouts) to stderr but still
    exits 0 and prints `"diagnostics": []`. A failed call is therefore
    indistinguishable from a clean bill of health unless stderr is inspected,
    which would silently score every rate-limited request as a non-detection.
    We treat any provider-level error on stderr as a failed call.
    """
    try:
        out = subprocess.run(
            [binary, *args], capture_output=True, text=True, env=env, timeout=timeout
        )
    except subprocess.TimeoutExpired:
        return None
    if out.returncode != 0 or not out.stdout.strip():
        return None
    err = out.stderr or ""
    if "Analysis request failed" in err or "ERROR" in err:
        return None
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return None


# The endpoint rejects bursts with "too many concurrent requests" even from a
# single client issuing one call at a time, so calls are paced, not just retried.
INTER_CALL_SECONDS = 20.0


def run_json_retry(binary, args, env=None, timeout=120, attempts=5, base_sleep=20.0):
    """run_json with pacing and exponential backoff, for the rate-limited provider."""
    time.sleep(INTER_CALL_SECONDS)
    for i in range(attempts):
        d = run_json(binary, args, env=env, timeout=timeout)
        if d is not None:
            return d
        if i < attempts - 1:
            time.sleep(base_sleep * (2 ** i))
    return None


def analyze_static(binary, prompt: Path):
    return run_json(
        binary,
        ["analyze", str(prompt), "--engine", "annotation", "--no-lint", "--format", "json"],
    )


def categories(binary, prompt: Path) -> dict[str, list[str]]:
    d = analyze_static(binary, prompt)
    cats: dict[str, list[str]] = {}
    if d:
        for n in d["ir"]["nodes"]:
            if n["kind"] == "Rule":
                c = n.get("meta", {}).get("category")
                if c:
                    cats.setdefault(c, []).append(n["id"])
    return cats


def static_unreachable(binary, prompt: Path) -> set[str]:
    """Ground-truth unreachable rule ids from the deterministic lint."""
    d = analyze_static(binary, prompt)
    ids: set[str] = set()
    if d:
        for diag in d.get("diagnostics", []):
            if diag.get("code") == "prompt/unreachable-rule":
                msg = diag.get("message", "")
                if msg.startswith("Rule "):
                    ids.add(msg.split()[1])
    return ids


def llm_diagnostics(binary, prompt: Path, env, with_ir: bool, timeout: int):
    """Raw LLM linter diagnostics, or None if the call failed."""
    args = [
        "analyze", str(prompt), "--engine", "annotation",
        "--no-static-lint", "--format", "json",
    ]
    if not with_ir:
        args.append("--ablate-ir")
    d = run_json_retry(binary, args, env=env, timeout=timeout)
    if d is None:
        return None
    return [
        {"code": x.get("code", ""), "message": x.get("message", "")}
        for x in d.get("diagnostics", [])
    ]


def _norm(diag) -> str:
    """Normalised key for deciding whether a diagnostic is 'the same one'."""
    return re.sub(r"\s+", " ", f"{diag['code']} {diag['message']}".strip().lower())


def _bare(rule_id: str) -> str:
    """`RULE:GENERIC_STYLE` -> `GENERIC_STYLE`.

    The LLM linter refers to rules by their bare name ("Rule ROLE does not
    specify a priority"), not by the fully-qualified id the IR uses. Matching
    only on `RULE:...` would score every genuine detection as a miss.
    """
    return rule_id.split(":", 1)[-1]


def classify(before, after, truth_delta: set[str], all_rule_ids: set[str]):
    """Compare one before/after diagnostic pair.

    Returns (strict_hit, lenient_hit, n_new, named_in_delta, named_outside_delta).
    """
    seen = {_norm(d) for d in before}
    new = [d for d in after if _norm(d) not in seen]

    delta_names = {_bare(r) for r in truth_delta}
    other_names = {_bare(r) for r in all_rule_ids} - delta_names

    named_in, named_out = set(), set()
    lenient = False
    for d in new:
        blob = f"{d['code']} {d['message']}"
        low = blob.lower()
        if any(k in low for k in DETECT_KEYWORDS):
            lenient = True
        # Rule names are uppercase identifiers; match on word boundaries so that
        # a short name like ROLE cannot be matched inside an ordinary word.
        tokens = set(re.findall(r"\b[A-Z][A-Z0-9_]{2,}\b", blob))
        tokens |= {t.split(":", 1)[-1] for t in RULE_TOKEN.findall(blob)}
        named_in |= tokens & delta_names
        named_out |= tokens & other_names

    return bool(named_in), lenient, len(new), named_in, named_out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=str(HERE / "corpus" / "synthetic"))
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--model", default="gemma3:12b")
    ap.add_argument("--timeout", type=int, default=110)
    ap.add_argument("--repeats", type=int, default=3)
    ap.add_argument("--out", default=str(HERE / "results" / "rq3_detection_v2.csv"))
    ap.add_argument("--raw-out", default=str(HERE / "results" / "rq3_raw"))
    ap.add_argument("--max", type=int, default=0)
    args = ap.parse_args()

    if not os.environ.get("OLLAMA_API_KEY") and "OPENAI_API_KEY" not in os.environ:
        sys.exit("Set OLLAMA_API_KEY (the LLM linter is required for RQ3).")

    binary = args.bin if Path(args.bin).exists() else "promptdbg"
    env = dict(os.environ, OLLAMA_MODEL=args.model)
    prompts = sorted(Path(args.corpus).glob("*.rtpl"))

    raw_dir = Path(args.raw_out)
    raw_dir.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="rq3v2_"))

    rows, kept = [], 0
    for p in prompts:
        if args.max and kept >= args.max:
            break
        cats = categories(binary, p)
        if not cats:
            continue

        cat = max(cats, key=lambda c: len(cats[c]))
        text = p.read_text()
        injected = tmp / p.name
        injected.write_text(
            f"// @rule RULE:INJECTED_DOMINATOR category={cat} priority=999\n" + text
        )

        all_rule_ids = {rid for ids in categories(binary, injected).values() for rid in ids}
        truth_before = static_unreachable(binary, p)
        truth_after = static_unreachable(binary, injected)
        truth_delta = truth_after - truth_before
        if not truth_delta:
            print(f"  {p.name}: injection created no new unreachable rule, skipping",
                  file=sys.stderr)
            continue

        per_cond = {}
        raw_log = {"prompt": p.name, "category": cat,
                   "truth_before": sorted(truth_before),
                   "truth_after": sorted(truth_after),
                   "truth_delta": sorted(truth_delta), "runs": []}
        failed = False

        for with_ir in (True, False):
            strict_hits, lenient_hits, fps = [], [], []
            for r in range(args.repeats):
                before = llm_diagnostics(binary, p, env, with_ir, args.timeout)
                after = llm_diagnostics(binary, injected, env, with_ir, args.timeout)
                if before is None or after is None:
                    continue
                s, l, n_new, n_in, n_out = classify(before, after, truth_delta, all_rule_ids)
                strict_hits.append(int(s))
                lenient_hits.append(int(l))
                fps.append(len(n_out))
                raw_log["runs"].append({
                    "with_ir": with_ir, "repeat": r,
                    "before": before, "after": after,
                    "strict": s, "lenient": l, "new_diagnostics": n_new,
                    "named_in_delta": sorted(n_in), "named_outside_delta": sorted(n_out),
                })
            if not strict_hits:
                failed = True
                break
            per_cond[with_ir] = (
                sum(strict_hits) / len(strict_hits),
                sum(lenient_hits) / len(lenient_hits),
                sum(fps) / len(fps),
                len(strict_hits),
            )

        if failed:
            print(f"  {p.name}: LLM lint failed for all runs, skipping", file=sys.stderr)
            continue

        (sw, lw, fw, nw) = per_cond[True]
        (so, lo, fo, no) = per_cond[False]
        rows.append({
            "prompt": p.name,
            "injected_category": cat,
            "truth_delta_size": len(truth_delta),
            "pre_existing_unreachable": len(truth_before),
            "runs_with_ir": nw, "runs_without_ir": no,
            "strict_with_ir": round(sw, 3), "strict_without_ir": round(so, 3),
            "lenient_with_ir": round(lw, 3), "lenient_without_ir": round(lo, 3),
            "false_named_with_ir": round(fw, 3), "false_named_without_ir": round(fo, 3),
        })
        (raw_dir / f"{p.stem}.json").write_text(json.dumps(raw_log, indent=2))
        kept += 1
        print(f"  {p.name}: delta={len(truth_delta)} pre={len(truth_before)} "
              f"strict={sw:.2f}/{so:.2f} lenient={lw:.2f}/{lo:.2f}", file=sys.stderr)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    fields = list(rows[0].keys()) if rows else ["prompt"]
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    print(f"\nWrote {len(rows)} labelled defects to {out}")
    if rows:
        import statistics as st
        n = len(rows)

        def ci95(xs):
            return 1.96 * st.pstdev(xs) / (n ** 0.5) if n >= 2 else 0.0

        for label, kw, ko in (("STRICT (names a newly-shadowed rule)", "strict_with_ir", "strict_without_ir"),
                              ("LENIENT (keyword, but still differential)", "lenient_with_ir", "lenient_without_ir")):
            xw = [r[kw] for r in rows]
            xo = [r[ko] for r in rows]
            d = [a - b for a, b in zip(xw, xo)]
            print(f"\n{label}, {args.repeats} runs/condition, n={n}:")
            print(f"  with IR:    {st.mean(xw):.3f} +/- {ci95(xw):.3f}")
            print(f"  without IR: {st.mean(xo):.3f} +/- {ci95(xo):.3f}")
            print(f"  paired diff: {st.mean(d):+.3f} +/- {ci95(d):.3f}")
        fw = [r["false_named_with_ir"] for r in rows]
        print(f"\nmean rules named that were NOT newly shadowed (with IR): {st.mean(fw):.2f}")
        print("static analyser recall = 1.00 by construction (it defines the ground truth)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
