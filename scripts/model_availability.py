#!/usr/bin/env python3
"""Audit whether the models this study measured are still queryable.

A study of an evaluation's reproducibility ought to check the most basic
precondition of all: can the experiment be run again at all? For a study
conducted against a commercial open-model endpoint, the answer has a shelf life,
because the provider decides which weights stay served.

This script asks the endpoint which models it currently offers, then probes each
model the study measured, recording the HTTP status and, for withdrawn models,
the retirement date the provider reports. The output is a dated artifact: rerun
it and the picture will differ, which is the point.

Usage:
    OLLAMA_API_KEY=... python3 scripts/model_availability.py --stamp 2026-08-17
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import time

import requests

BASE = "https://ollama.com/v1"

# Every model named in the study, with the role it played.
STUDIED = [
    ("gemma3:12b", "reported in Table 1"),
    ("gpt-oss:20b", "reported in Table 1"),
    ("gpt-oss:120b", "reported in Table 1"),
    ("ministral-3:8b", "reported in Table 1"),
    ("mistral-large-3:675b", "reported in Table 1"),
    ("minimax-m2.1", "reported in Table 1"),
    ("minimax-m2.7", "reported in Table 1"),
    ("qwen3-coder-next", "reported in Table 1"),
    ("deepseek-v3.1:671b", "attempted, no successful runs"),
    ("glm-4.7", "attempted, below inclusion threshold"),
    ("deepseek-v3.2", "probed as a substitute"),
    ("glm-4.6", "probed as a substitute"),
    ("nemotron-3-super", "probed as a substitute"),
    ("qwen3-next:80b", "probed as a substitute"),
    ("kimi-k2-thinking", "probed as a substitute"),
]


def catalogue(key: str) -> list[str]:
    r = requests.get(f"{BASE}/models", headers={"Authorization": f"Bearer {key}"}, timeout=60)
    r.raise_for_status()
    return sorted(m["id"] for m in r.json().get("data", []))


def probe(key: str, model: str) -> tuple[int, str]:
    """Minimal completion request. Returns (status, provider message)."""
    try:
        r = requests.post(
            f"{BASE}/chat/completions",
            headers={"Authorization": f"Bearer {key}"},
            json={"model": model, "messages": [{"role": "user", "content": "hi"}],
                  "max_tokens": 5},
            timeout=120,
        )
    except Exception as exc:  # network-level failure is itself a result
        return (-1, f"request failed: {exc}")
    if r.status_code == 200:
        return (200, "")
    try:
        return (r.status_code, r.json()["error"]["message"])
    except Exception:
        return (r.status_code, r.text[:200])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--stamp", required=True,
                    help="date of this audit, YYYY-MM-DD (scripts cannot read the clock "
                         "reproducibly, so the date is an explicit input)")
    ap.add_argument("--out", default=os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "results", "model_availability.csv"))
    args = ap.parse_args()

    key = os.environ.get("OLLAMA_API_KEY")
    if not key:
        sys.exit("Set OLLAMA_API_KEY.")

    offered = catalogue(key)
    print(f"endpoint currently offers {len(offered)} models:")
    for m in offered:
        print(f"   {m}")

    rows = []
    print(f"\n{'model':26}{'role':38}{'status':>8}  note")
    for model, role in STUDIED:
        status, note = probe(key, model)
        rows.append({
            "audit_date": args.stamp, "model": model, "study_role": role,
            "http_status": status, "available": int(status == 200),
            "provider_message": note,
        })
        print(f"{model:26}{role:38}{status:>8}  {note[:70]}")
        time.sleep(1)

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    table1 = [r for r in rows if r["study_role"] == "reported in Table 1"]
    alive = sum(r["available"] for r in table1)
    print(f"\nTable 1 models still queryable: {alive}/{len(table1)}")
    gone = [r["model"] for r in table1 if not r["available"]]
    if gone:
        print(f"withdrawn: {', '.join(gone)}")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
