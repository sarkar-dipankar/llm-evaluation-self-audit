#!/usr/bin/env python3
"""Were the models that "failed" actually inaccessible, or did our client give up?

An earlier version of this study reported that two model families returned no
usable inference and described the failures as *structural*, a property of those
families rather than of our setup. This script tests that attribution directly,
and finds it wrong.

Two properties of our harness explain the failures without invoking anything
about the models:

1. **A hardcoded 60-second client timeout.** `prompt_ir::config::
   default_analysis_timeout` returns 60000 ms, with no CLI or environment
   override. Any inference slower than that is recorded as a connection failure.

2. **An output-token budget that ignores reasoning traces.** Several current
   models return a separate `reasoning` field. The tokens spent there count
   against `max_tokens`, so with a modest budget the reasoning consumes the
   whole allowance, `finish_reason` comes back as `length`, and `content` is
   empty or truncated to invalid JSON. Our harness sees a parse failure.

The script measures, per model, the latency and the reasoning/content token
split under a small budget and under a generous one, and reports whether a
model that "fails" under our harness succeeds when given room. Every model that
succeeds in the second condition is one whose failure was ours, not theirs.

Usage:
    OLLAMA_API_KEY=... python3 scripts/failure_attribution.py --stamp 2026-08-17
"""
from __future__ import annotations

import argparse
import csv
import glob
import json
import os
import sys
import time

import requests

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
API = "https://ollama.com/v1/chat/completions"

# The client timeout compiled into the harness, in seconds.
HARNESS_TIMEOUT_S = 60.0

DEFAULT_MODELS = [
    "gpt-oss:20b", "gpt-oss:120b", "mistral-large-3:675b", "minimax-m2.7",
    "gemma4:31b", "qwen3.5:397b", "glm-5.2", "deepseek-v4-pro:0813",
    "nemotron-3-nano:30b", "nemotron-3-super", "nemotron-3-ultra",
]


def probe(key: str, model: str, prompt: str, max_tokens: int):
    t = time.time()
    try:
        r = requests.post(
            API, headers={"Authorization": f"Bearer {key}"},
            json={"model": model, "temperature": 0.0,
                  "messages": [{"role": "user", "content": prompt}],
                  "max_tokens": max_tokens},
            timeout=600,
        )
    except Exception as exc:
        return {"latency_s": time.time() - t, "status": -1, "error": str(exc)[:120]}
    el = time.time() - t
    if r.status_code != 200:
        return {"latency_s": el, "status": r.status_code, "error": r.text[:120]}
    d = r.json()
    choice = d["choices"][0]
    msg = choice["message"]
    content = msg.get("content") or ""
    reasoning = msg.get("reasoning") or ""
    valid = False
    if "{" in content and "}" in content:
        try:
            json.loads(content[content.index("{"): content.rindex("}") + 1])
            valid = True
        except Exception:
            valid = False
    return {
        "latency_s": el, "status": 200, "error": "",
        "finish_reason": choice.get("finish_reason", ""),
        "reasoning_chars": len(reasoning), "content_chars": len(content),
        "valid_json": int(valid),
        "completion_tokens": (d.get("usage") or {}).get("completion_tokens", 0),
    }


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--stamp", required=True, help="audit date YYYY-MM-DD")
    ap.add_argument("--models", default=",".join(DEFAULT_MODELS))
    ap.add_argument("--small-budget", type=int, default=1500)
    ap.add_argument("--large-budget", type=int, default=8000)
    ap.add_argument("--out", default=os.path.join(HERE, "results", "failure_attribution.csv"))
    args = ap.parse_args()

    key = os.environ.get("OLLAMA_API_KEY")
    if not key:
        sys.exit("Set OLLAMA_API_KEY.")

    sources = sorted(glob.glob(os.path.join(HERE, "corpus", "github-crossed", "*.md")))
    if not sources:
        sys.exit("no probe corpus at corpus/github-crossed/")
    src = open(sources[0], errors="ignore").read()
    prompt = ("Extract the structure of the following prompt as JSON with a "
              "'nodes' array. Respond with only JSON.\n\n---\n" + src + "\n---")

    rows = []
    print(f"{'model':24}{'budget':>8}{'lat(s)':>8}{'finish':>9}{'reason':>8}"
          f"{'content':>9}{'json':>6}  verdict")
    for model in args.models.split(","):
        model = model.strip()
        for label, budget in (("small", args.small_budget), ("large", args.large_budget)):
            res = probe(key, model, prompt, budget)
            would_time_out = res["latency_s"] > HARNESS_TIMEOUT_S
            usable = res.get("valid_json", 0) == 1 and not would_time_out
            verdict = ("ok" if usable else
                       "harness timeout" if would_time_out and res.get("valid_json") else
                       "timeout" if would_time_out else
                       "truncated by budget" if res.get("finish_reason") == "length" else
                       "other failure")
            rows.append({"audit_date": args.stamp, "model": model, "budget": budget,
                         "budget_label": label, **res,
                         "exceeds_harness_timeout": int(would_time_out),
                         "verdict": verdict})
            print(f"{model:24}{label:>8}{res['latency_s']:>8.1f}"
                  f"{str(res.get('finish_reason',''))[:8]:>9}"
                  f"{res.get('reasoning_chars',0):>8}{res.get('content_chars',0):>9}"
                  f"{res.get('valid_json',0):>6}  {verdict}")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=sorted({k for r in rows for k in r}))
        w.writeheader()
        w.writerows(rows)

    small = {r["model"]: r for r in rows if r["budget_label"] == "small"}
    large = {r["model"]: r for r in rows if r["budget_label"] == "large"}
    rescued = [m for m in small
               if not small[m].get("valid_json") and large[m].get("valid_json")]
    slow = [m for m in large if large[m]["exceeds_harness_timeout"]]
    print(f"\nmodels that fail under our harness budget but succeed with room: "
          f"{len(rescued)} ({', '.join(rescued) if rescued else 'none'})")
    print(f"models whose successful inference still exceeds the {HARNESS_TIMEOUT_S:.0f}s "
          f"client timeout: {len(slow)} ({', '.join(slow) if slow else 'none'})")
    print("\nEvery model in either list failed for a reason located in the harness, "
          "not in the model or the provider.")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
