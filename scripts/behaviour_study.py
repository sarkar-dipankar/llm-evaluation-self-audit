#!/usr/bin/env python3
"""Do model-visible prompt mutations actually change what an agent does?

Section "Operator visibility" establishes that some mutation operators change the
text a model receives and others cannot. That is a statement about the *input*.
It does not establish that a changed input produces a changed response, which is
what a mutation score is implicitly claiming when it is read as test adequacy.

This script closes that gap. For each sampled mutant it renders the original and
the mutant under the same context, discards the pair if the two rendered texts
are identical (nothing to detect), and otherwise queries a model R times on each.
The question is not "did the outputs differ" -- they always differ a little,
because the model is not deterministic. The question is whether they differ
*more across the mutation than they do across repeated runs of the same prompt*.

So each pair yields three quantities:

    within_original  mean pairwise similarity among the R original responses
    within_mutant    mean pairwise similarity among the R mutant responses
    between          mean similarity between original and mutant responses

and the mutant counts as behaviourally detected when `between` falls below both
within-condition similarities: the mutation moved the response distribution
further than run-to-run noise does. The noise floor is measured, not assumed,
which matters because the same endpoint's nondeterminism is the subject of our
companion study.

Usage:
    OLLAMA_API_KEY=... python3 scripts/behaviour_study.py --model gpt-oss:120b \
        --pairs 40 --repeats 3
"""
from __future__ import annotations

import argparse
import csv
import itertools
import json
import os
import random
import re
import subprocess
import sys
import time

import requests

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(HERE, "tool", "target", "release", "promptdbg")
CORPUS = os.path.join(HERE, "corpus", "synthetic")
CONTEXTS = os.path.join(HERE, "corpus", "contexts")
API = "https://ollama.com/v1/chat/completions"

# Operators that can change rendered text after the body-carrying repair.
MODEL_VISIBLE = ("DropRule", "SwapRules", "NegateCondition")

WORD = re.compile(r"[a-z0-9']+")
STOP = {
    "the", "a", "an", "and", "or", "but", "if", "then", "of", "to", "in", "on",
    "for", "with", "is", "are", "be", "as", "at", "by", "it", "this", "that",
}

# Realistic payloads keyed by variable name fragment, so the task the model is
# asked to perform is not the placeholder "x" the coverage generator emits.
PAYLOADS = {
    "diff": "def process(items):\n    for i in range(len(items)):\n        items[i] = items[i].strip()\n    return items",
    "body": "def process(items):\n    for i in range(len(items)):\n        items[i] = items[i].strip()\n    return items",
    "content": "This product is a complete waste of money and the seller should be ashamed.",
    "message": "This product is a complete waste of money and the seller should be ashamed.",
    "description": "Login page returns a 500 error after the latest deploy; affects all users.",
    "text": "Login page returns a 500 error after the latest deploy; affects all users.",
    "query": "How many orders shipped last week?",
    "question": "How many orders shipped last week?",
    "comment": "Great post, check out my site at example.com for cheap deals!",
    "ticket": "Login page returns a 500 error after the latest deploy; affects all users.",
    "email": "Hi, I was charged twice for my subscription this month. Please advise.",
}


def enrich(obj):
    """Replace placeholder string leaves with realistic content.

    Booleans and enum-like strings chosen by the coverage-directed generator are
    left alone, so the branch the context was built to exercise is preserved.
    """
    if isinstance(obj, dict):
        return {k: enrich_value(k, v) for k, v in obj.items()}
    return obj


def enrich_value(key, value):
    if isinstance(value, dict):
        return {k: enrich_value(k, v) for k, v in value.items()}
    if isinstance(value, str):
        if value.startswith("__") or len(value) > 40:
            return value  # sentinel or already substantial
        for frag, payload in PAYLOADS.items():
            if frag in key.lower():
                return payload
        return value
    return value


def render(path: str, context: dict) -> str | None:
    proc = subprocess.run(
        [BIN if os.path.exists(BIN) else "promptdbg", "render", path,
         "--context", json.dumps(context)],
        capture_output=True, text=True, timeout=120,
    )
    return proc.stdout if proc.returncode == 0 else None


def mutants_for(path: str):
    """(operator, id, source) for each mutant, from `mutate --list --format json`.

    The JSON listing carries each mutant's full source, so mutants are generated
    by the tool under test rather than reimplemented here.
    """
    proc = subprocess.run(
        [BIN if os.path.exists(BIN) else "promptdbg", "mutate", path,
         "--list", "--format", "json"],
        capture_output=True, text=True, timeout=120,
    )
    if proc.returncode != 0:
        return []
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return []
    return [(m["operator"], m["id"], m["source"]) for m in data]


def ask(key: str, model: str, prompt: str, timeout: int, max_tokens: int):
    """Return (text, finish_reason), or (None, reason) when unusable.

    Two response classes must be excluded rather than scored. A reply truncated
    by the token cap ends at an arbitrary point, so two truncated replies to the
    *same* prompt differ for a reason that has nothing to do with the mutation;
    including them inflates the run-to-run noise floor and, because the floor is
    the comparison baseline, biases the test toward finding no effect. An empty
    reply is a failed call, not a behaviour. Both are dropped, and the counts are
    reported so the exclusion is visible.
    """
    try:
        r = requests.post(
            API, headers={"Authorization": f"Bearer {key}"},
            json={"model": model, "temperature": 0.0,
                  "messages": [{"role": "user", "content": prompt}],
                  "max_tokens": max_tokens},
            timeout=timeout,
        )
    except Exception:
        return None, "exception"
    if r.status_code != 200:
        return None, f"http{r.status_code}"
    try:
        choice = r.json()["choices"][0]
        text = choice["message"].get("content") or ""
        finish = choice.get("finish_reason", "")
    except Exception:
        return None, "malformed"
    if finish == "length":
        return None, "truncated"
    if not text.strip():
        return None, "empty"
    return text, finish


def tokens(s: str) -> set:
    return {w for w in WORD.findall(s.lower()) if w not in STOP}


def sim(a: str, b: str) -> float:
    ta, tb = tokens(a), tokens(b)
    if not ta and not tb:
        return 1.0
    return len(ta & tb) / max(1, len(ta | tb))


def mean_pairwise(xs) -> float:
    ps = list(itertools.combinations(range(len(xs)), 2))
    if not ps:
        return 1.0
    return sum(sim(xs[i], xs[j]) for i, j in ps) / len(ps)


def mean_cross(xs, ys) -> float:
    return sum(sim(a, b) for a in xs for b in ys) / max(1, len(xs) * len(ys))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--model", default="gpt-oss:120b")
    ap.add_argument("--pairs", type=int, default=40)
    ap.add_argument("--repeats", type=int, default=3)
    ap.add_argument("--timeout", type=int, default=240)
    ap.add_argument("--max-tokens", type=int, default=1500,
                    help="must be generous enough that replies finish; truncated "
                         "replies are discarded, not scored")
    ap.add_argument("--seed", type=int, default=20260817)
    ap.add_argument("--out", default=os.path.join(HERE, "results", "behaviour_study.csv"))
    ap.add_argument("--raw-out", default=os.path.join(HERE, "results", "behaviour_raw"))
    args = ap.parse_args()

    key = os.environ.get("OLLAMA_API_KEY")
    if not key:
        sys.exit("Set OLLAMA_API_KEY.")
    rng = random.Random(args.seed)
    os.makedirs(args.raw_out, exist_ok=True)

    # Build the candidate pool: (prompt, context, mutant) triples whose rendered
    # texts actually differ.
    pool = []
    for name in sorted(os.listdir(CORPUS)):
        if not name.endswith(".prompt.rtpl"):
            continue
        stem = name[: -len(".prompt.rtpl")]
        cpath = os.path.join(CONTEXTS, f"{stem}.contexts.json")
        if not os.path.exists(cpath):
            continue
        try:
            ctxs = json.load(open(cpath))
        except Exception:
            continue
        if not ctxs:
            continue
        ctx = enrich(ctxs[0])
        path = os.path.join(CORPUS, name)
        base = render(path, ctx)
        if not base:
            continue
        for op, mid, src in mutants_for(path):
            if op not in MODEL_VISIBLE:
                continue
            pool.append((name, path, ctx, op, mid, src, base))

    rng.shuffle(pool)
    print(f"candidate model-visible mutants: {len(pool)}", file=sys.stderr)

    rows, done = [], 0
    tmpdir = os.path.join(args.raw_out, "_tmp")
    os.makedirs(tmpdir, exist_ok=True)

    for name, path, ctx, op, mid, src, base in pool:
        if done >= args.pairs:
            break
        # Materialise the mutant source the tool generated, then render it.
        mpath = os.path.join(tmpdir, "m.prompt.rtpl")
        with open(mpath, "w") as fh:
            fh.write(src)
        mrender = render(mpath, ctx)
        if not mrender or mrender == base:
            continue  # not model-visible under this context

        orig_outs, mut_outs, dropped = [], [], []
        for _ in range(args.repeats):
            a, ra = ask(key, args.model, base, args.timeout, args.max_tokens)
            b, rb = ask(key, args.model, mrender, args.timeout, args.max_tokens)
            if a is None or b is None:
                dropped.append(ra if a is None else rb)
                continue
            orig_outs.append(a)
            mut_outs.append(b)
        if len(orig_outs) < 3:
            print(f"  {name} {mid}: only {len(orig_outs)} usable run pairs "
                  f"({','.join(dropped) or 'none'}), skipping", file=sys.stderr)
            continue

        wo, wm = mean_pairwise(orig_outs), mean_pairwise(mut_outs)
        bt = mean_cross(orig_outs, mut_outs)
        detected = int(bt < min(wo, wm))
        rows.append({
            "prompt": name, "operator": op, "mutant": mid,
            "within_original": round(wo, 4), "within_mutant": round(wm, 4),
            "between": round(bt, 4),
            "margin": round(min(wo, wm) - bt, 4),
            "detected": detected, "repeats": len(orig_outs),
            "dropped_runs": len(dropped),
        })
        with open(os.path.join(args.raw_out, f"{name}.{mid.replace('/', '_')}.json"), "w") as fh:
            json.dump({"prompt": name, "mutant": mid, "operator": op,
                       "rendered_original": base, "rendered_mutant": mrender,
                       "original_responses": orig_outs, "mutant_responses": mut_outs}, fh, indent=2)
        done += 1
        print(f"  [{done}/{args.pairs}] {name} {mid}: within {wo:.3f}/{wm:.3f} "
              f"between {bt:.3f} -> {'DETECTED' if detected else 'not detected'}",
              file=sys.stderr)

    if not rows:
        sys.exit("no usable pairs; check the model name and API key")

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)

    import statistics as st
    print(f"\npairs measured: {len(rows)}  model: {args.model}  R={args.repeats}")
    det = sum(r["detected"] for r in rows)
    print(f"behaviourally detected: {det}/{len(rows)} ({100*det/len(rows):.0f}%)")
    print(f"mean within-condition similarity: "
          f"{st.mean([r['within_original'] for r in rows] + [r['within_mutant'] for r in rows]):.3f}")
    print(f"mean between-condition similarity: {st.mean(r['between'] for r in rows):.3f}")
    print(f"\n{'operator':18}{'n':>4}{'detected':>10}{'mean margin':>13}")
    by = {}
    for r in rows:
        by.setdefault(r["operator"], []).append(r)
    for op, rs in sorted(by.items()):
        d = sum(x["detected"] for x in rs)
        print(f"{op:18}{len(rs):>4}{f'{d}/{len(rs)}':>10}{st.mean(x['margin'] for x in rs):>13.4f}")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
