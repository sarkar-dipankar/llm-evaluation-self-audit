#!/usr/bin/env python3
"""Synthesize annotated `.prompt.rtpl` prompts to extend the corpus.

Generates structurally diverse, annotated prompts (rules, conditions, breakpoints,
and `{% if %}` branches) via any OpenAI-compatible chat endpoint. Defaults target
Ollama Cloud (``https://ollama.com/v1`` with ``OLLAMA_API_KEY``), so synthesis can
run on open models.

Usage:
  OLLAMA_API_KEY=... python scripts/synthesize.py --n 20 --model gpt-oss:120b
  OPENAI_API_KEY=... python scripts/synthesize.py --n 20 \
      --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY --model gpt-4o-mini
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
DEFAULT_BIN = HERE/ "tool" / "target" / "release" / "promptdbg"

try:
    import requests
except ImportError:
    sys.exit("This script needs `requests`: pip install requests")

SPEC = r"""You write `.prompt.rtpl` files for the PromptDbg toolchain. Format rules:
- Annotate behavioral rules with `// @rule RULE:NAME category=<style|safety|policy|format> [priority=N]`.
- Annotate conditions with `// @condition CONDITION:NAME`.
- Use `{% if expr %} ... {% else %} ... {% endif %}` control flow; `expr` may use
  dot paths and `==`/`!=` (e.g. `user.role == "admin"`).
- Optionally add `// @breakpoint BREAKPOINT:NAME [conditionRef=CONDITION:NAME]`.
- Use `{{ var.path }}` for interpolation ONLY. Do NOT use filters (`| length`,
  `| join`), method/property calls (`.length`, `.upper()`), or arithmetic — the
  engine supports only plain dot-path variables and `==`/`!=`/truthy conditions.
Produce ONE complete prompt for the given domain. At least 4 rules, 2 conditions,
some nested branching. Output ONLY the prompt file content, no commentary."""

DOMAINS = [
    "triage support tickets", "review SQL migrations", "moderate forum posts",
    "summarize legal contracts", "answer medical FAQs with disclaimers",
    "grade student essays", "route sales leads", "generate release notes",
    "screen job applications", "translate with tone control",
    "classify incoming emails", "draft incident postmortems", "review pull requests",
    "recommend products", "extract invoice fields", "detect spam comments",
    "answer HR policy questions", "plan a travel itinerary", "summarize meetings",
    "write SQL from questions", "diagnose build failures", "moderate chat messages",
    "grade code submissions", "route insurance claims", "compose marketing copy",
    "check accessibility issues", "redact PII from text", "rank search results",
    "answer tax questions with disclaimers", "suggest unit tests",
]


def slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_")


def generate(session, base_url, model, domain) -> str:
    resp = session.post(
        f"{base_url}/chat/completions",
        json={
            "model": model,
            "temperature": 0.7,
            "messages": [
                {"role": "system", "content": SPEC},
                {"role": "user", "content": f"Domain: {domain}."},
            ],
        },
        timeout=120,
    )
    resp.raise_for_status()
    text = resp.json()["choices"][0]["message"]["content"]
    # Strip markdown fences if present.
    text = re.sub(r"^```[a-zA-Z]*\n?|\n?```$", "", text.strip())
    return text.strip() + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--n", type=int, default=10)
    ap.add_argument("--out", default="corpus/synthetic")
    ap.add_argument("--model", default="gpt-oss:120b")
    ap.add_argument("--base-url", default="https://ollama.com/v1")
    ap.add_argument("--api-key-env", default="OLLAMA_API_KEY")
    ap.add_argument("--contexts-dir", default=str(HERE / "corpus" / "contexts"),
                    help="emit a deterministic <stem>.contexts.json per prompt here")
    ap.add_argument("--bin", default=str(DEFAULT_BIN), help="promptdbg binary for gen-contexts")
    ap.add_argument("--start", type=int, default=0, help="starting index (extends corpus)")
    args = ap.parse_args()

    key = os.environ.get(args.api_key_env)
    if not key:
        sys.exit(f"Set {args.api_key_env} to your API key.")

    session = requests.Session()
    session.headers.update({"Authorization": f"Bearer {key}"})

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    for j in range(args.n):
        i = args.start + j
        domain = DOMAINS[i % len(DOMAINS)]
        try:
            content = generate(session, args.base_url, args.model, domain)
        except Exception as exc:  # noqa: BLE001 - keep going on transient failures
            print(f"  [{i}] {domain}: {exc}", file=sys.stderr)
            continue
        stem = f"gen_{i:03d}_{slug(domain)}"
        path = out_dir / f"{stem}.prompt.rtpl"
        path.write_text(content)
        print(f"  wrote {path}")

        # Emit matching, coverage-directed input contexts (deterministic).
        binary = args.bin if Path(args.bin).exists() else "promptdbg"
        ctx_dir = Path(args.contexts_dir)
        ctx_dir.mkdir(parents=True, exist_ok=True)
        try:
            subprocess.run(
                [binary, "gen-contexts", str(path),
                 "--out", str(ctx_dir / f"{stem}.contexts.json")],
                check=True, capture_output=True, timeout=30,
            )
            print(f"  wrote {ctx_dir / f'{stem}.contexts.json'}")
        except Exception as exc:  # noqa: BLE001
            print(f"  (gen-contexts skipped: {exc})", file=sys.stderr)

    print(f"\nDone. Validate with: promptdbg analyze <file> --engine annotation")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
