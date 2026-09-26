# LLM evaluation self-audit

Public reproducibility artifact for **How Reproducible Are Evaluation Conclusions? A Self-Audit of LLM-Inferred Prompt Structure**, by **Dipankar Sarkar, Skelf Research**.

- Paper: https://arxiv.org/abs/2609.30074
- DOI: https://doi.org/10.48550/arXiv.2609.30074
- Repository: https://github.com/sarkar-dipankar/llm-evaluation-self-audit

## What this repository contains

This release starts from the exact source and ancillary package submitted for arXiv v1. The analysis data and code are preserved; this README and citation metadata make the release easier to find and cite. The source-package SHA-256 is `6297e6cd7f749852d5ecdafa9e0766fad319324cc239dd8cc1da8351e5942fb3`.

| Directory | Contents |
|---|---|
| `paper/` | LaTeX manuscript source, bibliography, and style |
| `scripts/` | Analysis and collection scripts |
| `results/` | Measurements and persisted raw model outputs |
| `corpus/` | Synthetic and public-repository prompts, contexts, annotations, and provenance manifest |
| `tool/` | Rust analysis workspace and the recorded operator-repair patch |

Most analyses replay the saved outputs offline. The artifact also contains companion prompt-mutation experiments; the historical [artifact guide](ARTIFACT_GUIDE.md) covers that broader package and uses the original experiment numbering.

## Reproduce the self-audit

Use Python 3 and install the dependencies into a virtual environment:

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install -r requirements.txt
python scripts/merge_ir_stability.py
python scripts/stats_analysis.py
python scripts/drift_decompose.py
python scripts/downstream_cost.py
python scripts/ir_correctness.py
python scripts/stability_vs_correctness.py
```

Run commands from the repository root. Scripts may overwrite their derived result files. See the manuscript and each script for inputs, assumptions, and output interpretation.

The live endpoint checks in `model_availability.py` and `failure_attribution.py` are optional, require provider access, and cannot recreate retired endpoints. Four of the eight evaluated model variants were no longer served at the paper's audit date. Preserved responses enable offline analysis; they do not make the original collection process repeatable.

For Rust-based companion experiments, build the workspace in `tool/` and follow [ARTIFACT_GUIDE.md](ARTIFACT_GUIDE.md).

## Provenance and reuse

The arXiv paper and original author-contributed materials were released with the arXiv submission under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Third-party prompt files and style files retain their original terms; `corpus/MANIFEST.csv` records prompt-source repositories, revisions, URLs, and licenses. Do not interpret the paper license as replacing third-party terms.

The repository contains a release snapshot, not the private working repository or its history. Raw evidence and scientific claims are unchanged from the published artifact.

## Citation

```bibtex
@misc{sarkar2026reproducible,
  title = {How Reproducible Are Evaluation Conclusions? A Self-Audit of LLM-Inferred Prompt Structure},
  author = {Dipankar Sarkar},
  year = {2026},
  eprint = {2609.30074},
  archivePrefix = {arXiv},
  primaryClass = {cs.CL},
  doi = {10.48550/arXiv.2609.30074},
  url = {https://arxiv.org/abs/2609.30074}
}
```
