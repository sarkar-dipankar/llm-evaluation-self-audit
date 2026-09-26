# Pre-repair measurements

The mutation results as they stood **before** the operator repairs described in the paper. They are
*measured*, not reconstructed: reverse-applying `tool/repairs.patch` to `tool/` yields exactly the
implementation that produced them.

## Reproducing

From the artifact root:

```sh
cp -r tool tool-prerepair
cd tool-prerepair && patch -p1 -R < repairs.patch && cargo build --release && cd ..
python3 scripts/operator_visibility.py \
    --bin tool-prerepair/target/release/promptdbg \
    --out /tmp/prerepair.csv
diff /tmp/prerepair.csv results/pre_repair/operator_visibility.csv
```

The diff is empty. This regenerates `operator_visibility.csv` exactly, including the 200
rendered-input kills, which is a measurement rather than a derivation.

| File | What it is |
|---|---|
| `operator_visibility.csv` | **The "before" column of the paper's operator table.** 1,644 mutants, 1,076 trace, 200 rendered-input; 81% of trace kills invisible at the rendered-input layer, a 5.38x oracle gap. |
| `operator_visibility_intermediate.csv` | An *intermediate* state, kept only so the two are not confused: after the body-carrying `DropRule`/`SwapRules` fix but before the same-guard restriction (1,644 / 1,246 / 797). **Not the "before" column.** |
| `bench.csv` | `bench.csv` at the pre-repair state (1,644 mutants, 1,076 killed). Byte-identical to `bench_original_committed.csv`. |
| `bench_original_committed.csv` | The same measurement under its original name; same MD5 as `bench.csv`. |
| `rq2_baseline.csv` | RQ2 before the same-guard restriction. |
| `ablation_oracle.csv` | Oracle ablation at the same point. |

## What the repairs changed

`crates/prompt_runtime/src/mutate.rs` (block extent, `swap_blocks`, the same-guard filter) and
`crates/prompt_template/src/lib.rs` (shared block scanners, and rejection of a second `{% else %}`
in one block).
