# Release validation

Validated on 26 September 2026.

- Source: the original arXiv submission archive, SHA-256 `6297e6cd7f749852d5ecdafa9e0766fad319324cc239dd8cc1da8351e5942fb3`.
- All original artifact files are byte-identical to that archive except the root README. The paper source adds the public repository URL; no results or scientific claims change.
- `python3 scripts/stats_analysis.py --boot 10000 --seed 20260817` successfully replayed the reported rank-stability and merge-sensitivity results: 9,583 usable replicates, 417 discarded; 98.8%/86.2% rank retention for the bottom two models; 68.0%/68.3% for the top two; four of eight rows change under merge precedence and the headline changes by 7.1 percentage points. The archived JSON output was preserved after the check.
- This run validates those bootstrap and merge results, not every test or collection script in the artifact.
- The linked manuscript compiled to 13 pages and all pages were visually reviewed.
- A credential-pattern and local-path scan found no actual credentials or private local paths. Its two matches were public-corpus documentation examples.
