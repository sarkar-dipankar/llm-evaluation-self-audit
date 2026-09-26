#!/usr/bin/env python3
"""Collect prompt/skill files from GitHub for the corpus.

Searches public GitHub for candidate prompt artifacts (``SKILL.md`` files and
``.prompt.rtpl`` / ``.prompt`` templates), keeps only files from repositories under
a permissive license, deduplicates by content hash, and writes an attribution
manifest. Downloaded files land in ``corpus/github/``.

Requirements:
  - A GitHub token in ``GITHUB_TOKEN`` (code search requires authentication).
  - ``pip install requests``

This script is rate-limited by the GitHub Search API (30 req/min authenticated).
It is intentionally conservative: license-gating and attribution are mandatory so
the resulting corpus is redistributable.

Usage:
  GITHUB_TOKEN=... python scripts/collect_github.py --max 200
"""
from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import os
import sys
import time
from pathlib import Path

try:
    import requests
except ImportError:
    sys.exit("This script needs `requests`: pip install requests")

# SPDX ids we consider redistributable with attribution.
PERMISSIVE = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC",
    "0BSD", "Unlicense", "CC0-1.0", "CC-BY-4.0", "CC-BY-3.0",
}

# Search queries: (query, label). Code search matches file paths/extensions.
QUERIES = [
    ("filename:SKILL.md", "skill"),
    ("extension:rtpl", "rtpl"),
    ("filename:system_prompt.txt", "system_prompt"),
]

API = "https://api.github.com"


def gh(session: requests.Session, url: str, **kwargs) -> requests.Response:
    """GET with basic secondary-rate-limit backoff."""
    for attempt in range(6):
        resp = session.get(url, **kwargs)
        if resp.status_code == 403 and "rate limit" in resp.text.lower():
            wait = 2 ** attempt
            print(f"  rate-limited, sleeping {wait}s", file=sys.stderr)
            time.sleep(wait)
            continue
        return resp
    resp.raise_for_status()
    return resp


def repo_license(session: requests.Session, full_name: str, cache: dict) -> str | None:
    if full_name in cache:
        return cache[full_name]
    resp = gh(session, f"{API}/repos/{full_name}/license")
    spdx = None
    if resp.status_code == 200:
        spdx = (resp.json().get("license") or {}).get("spdx_id")
    cache[full_name] = spdx
    return spdx


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--max", type=int, default=100, help="max files to keep")
    ap.add_argument("--out", default="corpus/github", help="download directory")
    ap.add_argument("--manifest", default="corpus/MANIFEST.csv")
    args = ap.parse_args()

    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        sys.exit("Set GITHUB_TOKEN (a GitHub PAT) to use the Search API.")

    session = requests.Session()
    session.headers.update({
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    license_cache: dict[str, str | None] = {}
    seen_hashes: set[str] = set()
    rows: list[dict] = []

    for query, label in QUERIES:
        if len(rows) >= args.max:
            break
        page = 1
        while len(rows) < args.max:
            resp = gh(
                session,
                f"{API}/search/code",
                params={"q": query, "per_page": 50, "page": page},
            )
            if resp.status_code != 200:
                print(f"  search '{query}' p{page}: HTTP {resp.status_code}", file=sys.stderr)
                break
            items = resp.json().get("items", [])
            if not items:
                break
            for item in items:
                if len(rows) >= args.max:
                    break
                full_name = item["repository"]["full_name"]
                spdx = repo_license(session, full_name, license_cache)
                if spdx not in PERMISSIVE:
                    continue
                # Fetch file content (base64).
                cresp = gh(session, item["url"])
                if cresp.status_code != 200:
                    continue
                payload = cresp.json()
                if payload.get("encoding") != "base64":
                    continue
                content = base64.b64decode(payload["content"])
                digest = hashlib.sha256(content).hexdigest()
                if digest in seen_hashes:
                    continue
                seen_hashes.add(digest)

                fname = f"{digest[:12]}_{Path(item['path']).name}"
                (out_dir / fname).write_bytes(content)
                rows.append({
                    "file": fname,
                    "label": label,
                    "repo": full_name,
                    "path": item["path"],
                    "commit": item.get("sha", ""),
                    "html_url": item.get("html_url", ""),
                    "license": spdx,
                    "sha256": digest,
                })
                print(f"  kept {full_name}/{item['path']} [{spdx}]")
            page += 1
            time.sleep(2)  # be gentle with the search API

    manifest = Path(args.manifest)
    manifest.parent.mkdir(parents=True, exist_ok=True)
    with manifest.open("w", newline="") as f:
        writer = csv.DictWriter(
            f,
            fieldnames=["file", "label", "repo", "path", "commit", "html_url", "license", "sha256"],
        )
        writer.writeheader()
        writer.writerows(rows)

    print(f"\nKept {len(rows)} files. Manifest: {manifest}")
    print("Every entry is license-gated and attributed (repo/path/commit/license).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
