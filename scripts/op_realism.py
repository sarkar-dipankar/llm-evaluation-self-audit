#!/usr/bin/env python3
"""Operator realism: mine the commit history of the 40 license-gated GitHub
`SKILL.md` files in MANIFEST.csv, classify diffs by the closest match to our
mutation-operator taxonomy, and report the distribution.

This is the missing realism check called for by Limitations: our operators
should reflect the kinds of edits authors actually make. We classify each
diff hunk as:

  DropRule         : a heading/bullet line (rule-like) was removed
  AddRule          : a heading/bullet line was added (out-of-operator-set;
                     tracked separately as "non-operator structural change")
  ChangeWording    : a kept rule's text was reworded (closest analogue: edits
                     to category/trigger metadata in our annotated format)
  Reorder          : the same set of rule lines appears in a different order

We use the `gh` CLI for the GitHub API (token from `gh auth token`).

  python scripts/op_realism.py --max-repos 40
"""
from __future__ import annotations

import argparse
import csv
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
RULE_RE = re.compile(r"^#+\s+(.+)$|^[-*+]\s+(.+)$|^\d+\.\s+(.+)$|^[A-Z][A-Za-z][^:.\n]{2,80}:\s*$")


def gh(args: list[str]) -> dict | list | None:
    try:
        out = subprocess.run(["gh", "api", *args],
                             capture_output=True, text=True, timeout=30)
    except FileNotFoundError:
        sys.exit("gh CLI is required; install and run `gh auth login`.")
    if out.returncode != 0:
        return None
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return None


def commits_for_path(repo: str, path: str) -> list[dict]:
    items = gh([f"/repos/{repo}/commits?path={path}&per_page=100"])
    return items if isinstance(items, list) else []


def file_at(repo: str, sha: str, path: str) -> str | None:
    """Fetch the raw file content at a specific sha. gh prints raw text to stdout
    when Accept: vnd.github.raw is set; otherwise returns base64 JSON."""
    try:
        out = subprocess.run(
            ["gh", "api", f"/repos/{repo}/contents/{path}?ref={sha}",
             "-H", "Accept: application/vnd.github.raw"],
            capture_output=True, text=True, timeout=30,
        )
    except FileNotFoundError:
        return None
    if out.returncode != 0:
        return None
    # If the body parses as JSON with base64 content, decode; otherwise it's raw.
    try:
        data = json.loads(out.stdout)
        if isinstance(data, dict) and data.get("encoding") == "base64":
            import base64
            return base64.b64decode(data.get("content", "")).decode("utf-8", "replace")
    except json.JSONDecodeError:
        pass
    return out.stdout


def rule_lines(text: str) -> list[str]:
    """Heuristic: rule-like lines are headings or imperative one-liners."""
    return [ln.strip() for ln in text.splitlines() if RULE_RE.match(ln.strip())]


def classify(prev: list[str], curr: list[str]) -> dict:
    set_prev, set_curr = set(prev), set(curr)
    dropped = set_prev - set_curr
    added = set_curr - set_prev
    reordered = (set_prev == set_curr) and (prev != curr) and bool(prev)
    kept = set_prev & set_curr
    return {
        "DropRule": len(dropped),
        "AddRule": len(added),
        "Reorder": int(reordered),
        "ChangeWording": int(bool(kept) and (len(prev) == len(curr)) and (prev != curr) and not reordered),
    }


def line_delta(prev_text: str, curr_text: str) -> dict:
    """Body-line delta: counts added/removed non-empty lines between versions."""
    prev_lines = {ln.strip() for ln in prev_text.splitlines() if ln.strip()}
    curr_lines = {ln.strip() for ln in curr_text.splitlines() if ln.strip()}
    return {
        "lines_added": len(curr_lines - prev_lines),
        "lines_removed": len(prev_lines - curr_lines),
        "lines_unchanged": len(prev_lines & curr_lines),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--manifest", default=str(HERE / "corpus" / "MANIFEST.csv"))
    ap.add_argument("--max-repos", type=int, default=40)
    ap.add_argument("--out", default=str(HERE / "results" / "op_realism.csv"))
    args = ap.parse_args()

    rows = list(csv.DictReader(Path(args.manifest).open()))[: args.max_repos]
    classified = []
    repos_with_history = 0
    revisions_total = 0

    for r in rows:
        repo, path = r["repo"], r["path"]
        commits = commits_for_path(repo, path)
        if len(commits) <= 1:
            continue
        repos_with_history += 1
        # walk commits oldest -> newest, compare consecutive pairs.
        ordered = list(reversed(commits))  # API returns newest first
        prev_text = None
        for c in ordered:
            sha = c["sha"]
            text = file_at(repo, sha, path)
            if text is None:
                continue
            if prev_text is not None:
                prev = rule_lines(prev_text)
                curr = rule_lines(text)
                tally = classify(prev, curr)
                delta = line_delta(prev_text, text)
                revisions_total += 1
                classified.append({
                    "repo": repo, "path": path, "sha": sha[:7],
                    **tally,
                    **delta,
                })
            prev_text = text
        print(f"  {repo}: {len(commits)} commits", file=sys.stderr)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if classified:
        with out.open("w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(classified[0].keys()))
            w.writeheader()
            w.writerows(classified)

    print(f"\nRepos with edit history (>1 commit on the file): {repos_with_history}/{len(rows)}")
    print(f"Total revisions analysed: {revisions_total}")
    if classified:
        revs_with_struct = sum(1 for c in classified if c["DropRule"] or c["AddRule"] or c["Reorder"])
        sums = {k: sum(c[k] for c in classified) for k in
                ("DropRule", "AddRule", "Reorder", "ChangeWording")}
        total_struct = sums["DropRule"] + sums["AddRule"] + sums["Reorder"]
        total_body_add = sum(c["lines_added"] for c in classified)
        total_body_rem = sum(c["lines_removed"] for c in classified)
        print(f"Revisions with structural rule-line change: {revs_with_struct}/{revisions_total}")
        if total_struct:
            for k in ("DropRule", "AddRule", "Reorder", "ChangeWording"):
                v = sums[k]
                pct = 100 * v / max(1, total_struct + sums["ChangeWording"])
                print(f"  {k:<14}: {v:5d}  ({pct:.1f}%)")
        print(f"Body-line additions across all revisions: {total_body_add}")
        print(f"Body-line removals across all revisions: {total_body_rem}")
        # Derive the headline from the data rather than asserting it: an earlier
        # version printed a fixed "53% of files" regardless of what was computed.
        files_with_history = len({c["repo"] + "/" + c["path"] for c in classified})
        pct_files = 100.0 * files_with_history / max(1, len(rows))
        print(f"\nFiles with more than one revision: {files_with_history}/{len(rows)} "
              f"({pct_files:.0f}%)")
        print("The operator-bucket classification is conservative: these SKILL.md files")
        print("use prose and headings rather than the annotated // @rule format, so a")
        print("rule is identified heuristically and wording-only edits are undercounted.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
