# Hand-annotated real-world agent prompts

This directory contains 12 of the 40 real-world agent skill prompts in `corpus/github/`,
hand-annotated with the `// @rule` / `// @condition` / `// @breakpoint` grammar and, where
the source genuinely expresses one, `{% if %}` control flow.

It exists to close a construct-validity gap. The 40 collected GitHub prompts carry **zero**
annotations and **zero** control flow, so before this work the coverage and mutation
definitions applied to none of them and all reported results came from synthesised prompts.

Provenance (repo, path, commit, licence, sha256) for every source file is in
`corpus/MANIFEST.csv`, keyed by the same basename.

---

## 1. Headline findings (read this first)

These are the honest results of the exercise, including the ones that are inconvenient.

1. **Real agent prompts are rule-dense but almost branch-free.**
   Over 4,323 source lines we found 455 genuine behavioural rules — a rule density
   indistinguishable from the synthetic corpus (468 rules over 4,127 lines). But we found
   only **9 conditionals that the template grammar can express**, versus 158 in the
   synthetic corpus over a comparable line count. That is a ~34x difference in branching
   density (1 condition per 480 real lines vs 1 per 26 synthetic lines).

   | corpus | files | lines | rules | conditions | `{% if %}` | `{% else %}` | `{% for %}` |
   |---|---:|---:|---:|---:|---:|---:|---:|
   | `corpus/synthetic/` | 83 | 4,127 | 468 | 158 | 307 | 193 | 25 |
   | `corpus/github-annotated/` | 12 | 4,323 | 455 | 9 | 9 | 4 | 0 |

2. **6 of the 12 files have no template-expressible conditional at all.** Their annotated
   form is flat: rules, no branches. We did not invent structure to avoid this.

3. **Not a single `{% for %}` loop.** Two files contain "for each ..." instructions
   (`8ad9c110c8fc` "For each enabled channel", `ab8bd009141a` "For each metric x group x
   time period"), but both iterate over a collection the agent itself discovers at run
   time, not over an input supplied to the prompt. Neither is a template loop.

4. **Real prompts express conditionality as run-time instructions, not template
   branching.** The dominant form is "when the tool returns X, do Y" — the prompt hands the
   model the whole decision procedure and the model evaluates the guard while it works.
   Template branching, by contrast, requires a guard the *renderer* can evaluate before the
   model sees anything. `541a1bc0d709` is the extreme case: it is the most rule-dense file
   in the corpus (70 rules in 173 lines) and contains an eight-row decision table, yet it
   yields zero template branches, because every guard depends on the output of a command
   the agent runs mid-loop.

5. **Multi-way dispatch is the most common conditional shape, and the grammar cannot
   express it.** We counted ~14 multi-way dispatch constructs (3+ mutually exclusive cases:
   command-routing tables, HTTP-status tables, priority ladders, an 11-way `mode:`
   dispatch). The renderer has no `elif`; encoding these as nested `if`/`else` chains would
   have been an editorial re-interpretation of a flat first-match table, so we left them as
   prose rules. If the paper wants real prompts to branch, `elif` (or a `match`) is the
   single highest-value grammar addition.

6. **Guards in real prompts are frequently conjunctive.** e.g. "If this is the user's first
   time **and** no `strategy.md` exists". The grammar supports one dot-path with optional
   `==`/`!=`/negation, so such guards are not liftable at all.

7. **Structural coverage saturates trivially on real prompts.** Because 7 of 12 files have
   no branches, a single generated context reaches 100% rule coverage; the whole set
   reaches 100% rule, condition and branch-outcome coverage from 21 generated contexts
   (mean 1.75 per file). The synthetic corpus needs far more. Coverage is therefore a much
   weaker discriminator on real content than the synthetic results suggest, and the paper
   should say so.

8. **Much behavioural content lives in markdown tables**, which the annotation grammar can
   only address at whole-table granularity in the general case (see §5).

9. **Half the collected corpus is reference documentation rather than behavioural prompt.**
   Across all 40 GitHub files, 40.3% of lines sit inside fenced code blocks or markdown
   tables, and **20 of the 40 files have at least 40% of their lines in that category**.
   `09e319ab5af3` (27 lines) contains no agent-directed imperative at all; it is an API
   contract, and is included deliberately as the honest floor of the distribution: 3 rules,
   0 conditions. `ccb5349ebdcc` is the same phenomenon at scale: 1,388 lines yielding 43
   rules, because ~85% of it documents HTTP endpoints.

### Limitations of this sample

- **The 12 are mildly biased toward instruction-bearing files.** Fence-and-table content is
  27.5% of the selected 12 against 40.3% of the full 40, because representativeness was
  balanced against having something to annotate. The rule densities reported here are
  therefore an *upper* bound for the corpus as a whole; the branch densities, if anything,
  are also an upper bound for the same reason.
- **12 of 40 is a sample, not a census.** Extrapolating the 9-conditions figure to the full
  corpus is not warranted; what is warranted is the qualitative claim that liftable
  conditionals are rare and that the blocking factors (run-time guards, multi-way dispatch,
  conjunctive guards, clause-level scope) are systematic rather than incidental.
- **The lifting criteria in §5 are a judgement, not a measurement.** A stricter reading
  (refusing the disjunctive guard in `9deacc979622`, or requiring an explicit `else` arm)
  gives 8 conditions; a looser one (lifting clause-level guards, or nesting multi-way
  dispatch into `if`/`else` chains) could give 30+. Every individual call is recorded in §8
  so a reviewer can re-derive either number.
- **Single annotator.** No second rater, so no inter-annotator agreement figure. The
  criteria in §3–§5 are written to be mechanically re-appliable, and the generator spec is
  line-addressed, so a replication is cheap.

---

## 2. Fidelity guarantee

The annotated files preserve their sources **byte for byte**. Every annotated file was
produced by inserting whole new lines into the source; no source line was edited,
reordered, reflowed or deleted, and **no `{{ }}` interpolation was introduced** (see §6).

Verification — this must print `OK` for all 12:

```bash
cd corpus/github-annotated
for f in *.prompt.rtpl; do
  stem="${f%.prompt.rtpl}"
  grep -vE '^[[:space:]]*(// @|\{% (if |else %\}|endif %\}|for |endfor %\}))' "$f" \
    | cmp -s - "../github/$stem.md" && echo "OK   $stem" || echo "FAIL $stem"
done
```

Because the renderer drops `// @` lines from its output, the *rendered* prompt for a
branch-free file is identical to the original source text. For the five files with
branches, the rendered prompt is the source minus the untaken arm.

---

## 3. Category vocabulary

Five categories, matching the three hand-authored seeds in `corpus/synthetic/`
(`code_review`, `content_moderation`, `customer_support`) so the two corpora are
comparable. Every rule carries exactly one.

| category | meaning | test |
|---|---|---|
| `style` | how the output should sound or read: persona, voice, tone, register, phrasing | changing it changes the *feel*, not the *content* |
| `format` | the shape of the output: structure, sections, fields to show, output language, markup | checkable against the artefact's form |
| `policy` | domain and business rules: what the agent must or must not do, scope limits, precedence between alternatives, tool-usage prohibitions | violating it produces a wrong-but-well-formed answer |
| `safety` | rules whose violation risks concrete harm: secrets and credentials, PII, destructive actions, authorisation, fabricated data, legal/medical disclaimers | violating it can hurt someone or leak something |
| `process` | procedure: order of steps, which tool to call, when to stop or wait, checkpoints, preconditions | about *how the work proceeds*, not what it produces |

Distribution over the 455 rules: `process` 216, `policy` 111, `format` 56, `safety` 38,
`style` 34.

Borderline calls we made consistently:

- Prohibitions on committing secrets, exposing tokens, running against unauthorised
  systems, fabricating data, or publishing without consent → `safety`, not `policy`.
- "Reply in the user's language" → `format` (it constrains the artefact), not `style`.
- "Read file X before doing Y" → `process`, even when X contains policy.
- Refusals ("I cannot predict short-term prices") → `safety`, because they exist to prevent
  harm from over-claiming.

## 4. Priority convention

`priority=` is assigned **only where the source itself expresses precedence**. Absence of a
priority means the source expressed none — it does not mean "low". 338 of the 455 rules
carry no priority; the 117 that do break down as 10x`10`, 30x`20`, 74x`30`, 2x`40`, 1x`50`.

Qualitative levels:

| value | assigned when the source ... |
|---|---|
| `30` | marks the rule as overriding or non-negotiable relative to others: "HARD RULE", "Critical", "IMPORTANT", "No exceptions", "最重要", "必须 ... 不可跳过", "重要提示" |
| `20` | places it ahead of others in an explicit order, or states it as a precondition: "run first", "Pre-check", "before X", "must complete before", "先", "Preferred" |
| `10` | marks it as a fallback or lower-ranked alternative: "Alternative", "Otherwise", "if unavailable, instead", "备选", "无法判断时" |

Explicitly ranked lists: when a source numbers alternatives by priority (e.g.
`### 优先级1 / 优先级2 / 优先级3`, or "我追求的（按优先级）" with five ranked items), rank *r*
of *n* maps to `priority = 10 * (n - r + 1)`. This is why `b1950a5dde10` contains
`priority=50` and `priority=40` and `9deacc979622` contains `priority=40`: those files
rank five and four alternatives respectively. Within a file the ordering is faithful;
across files only the qualitative levels 10/20/30 are comparable.

## 5. When a natural-language conditional became `{% if %}`

This is the central judgement of the exercise, so the criteria are explicit. A conditional
in the source was lifted into template control flow **only if all four hold**:

- **C1 — context-resolvable guard.** The guard is a property of the *invocation context*
  (the user's request, the data or files supplied, the environment/configuration) that a
  harness could evaluate *before* rendering. Guards resolved by the agent's own
  mid-execution observations (a command's output, a CI result, a classification the agent
  just made) are **not** lifted: they are instructions to the model, not renderer inputs.
- **C2 — block-scoped.** The source itself delimits the guarded region — a section under a
  heading, an explicitly labelled alternative, or an explicit `if:`/`if not:` pair of
  blocks. A guard attached to a single clause, or to one step inside a numbered procedure,
  stays as prose. This also guarantees that lifting inserts only new lines (§2).
- **C3 — expressible.** The guard maps onto the supported fragment: one dot-path,
  optionally compared with `==`/`!=` to a literal, or negated. Conjunctions or disjunctions
  over *distinct* properties are not lifted. A disjunction that enumerates values of a
  *single* underlying property ("beginner, non-programmer, or unsure") may be modelled as
  one boolean path; every such case is flagged per-file below.
- **C4 — binary.** `if`, optionally with `else`. Multi-way dispatch (3+ mutually exclusive
  cases) is **not** lifted: the renderer has no `elif`, and a nested `if`/`else` chain would
  misrepresent a flat first-match table as a hierarchy.

Two further mechanical constraints shaped the annotations:

- **Fenced code blocks are never annotated internally.** An annotation line inside a
  ``` fence would appear in the source as prompt content to a human reader even though the
  renderer strips it. Rules that describe a code block are attached to the sentence or
  heading that introduces it.
- **Markdown tables.** The renderer strips `// @` lines, so annotating individual table
  rows is *output*-safe, but it makes the source unreadable. We annotate per row only when
  each row is itself a self-contained behavioural rule the agent must apply — this happens
  exactly once, for the eight-row Decision Table in `541a1bc0d709`, which is that file's
  core behavioural content. Lookup tables (pricing, macro reference, event types, command
  routing) get one rule attached to the table's introducing line.

Breakpoints (`// @breakpoint`) were added only where the source states an explicit stop or
gate ("DO NOT proceed until ...", "ALWAYS wait for explicit user response", "Ask user to
confirm before writing", "halt with ..."). 13 in total.

## 6. What we deliberately did *not* do

- **No `{{ }}` interpolations were added.** The sources contain no variable slots in our
  grammar's sense. What they do contain — 11 of the 12 files — are ad-hoc placeholder
  conventions: `$ARGUMENTS`, `$GEN`, `$HUB`, `$CWD`, `<slug>`, `<skill-path>`, `<hub-path>`,
  `<filename>`, `<hex_string>`. Converting any of these to `{{ }}` would have edited the
  prompt text, so we left them alone. This is itself a finding: real skill prompts are
  parameterised, but by harness-specific conventions the grammar does not model.
- **No text was rewritten** to make a conditional liftable, to split a compound guard, or
  to turn a table into a list.
- **No rules were invented** for descriptive prose. Statements of fact ("The daemon runs as
  a background Node.js process") are not annotated; only instructions the agent could
  violate are.

## 7. Selection rationale

The 12 were chosen to span the corpus rather than to be convenient. The corpus ranges from
27 to 1,388 lines; the sample includes both extremes and covers each length quartile.

| # | file | src lines | quartile | domain | structural type | why chosen |
|---|---|---:|---|---|---|---|
| 1 | `09e319ab5af3_skill` | 27 | Q1 | weather / MCP tool | API reference, no frontmatter | shortest file in the corpus; the honest floor — reference documentation with no agent-directed imperative |
| 2 | `194e3e6b9421_SKILL` | 73 | Q1 | marketing | router / orchestrator | densest conditional language among short files; a pure routing skill |
| 3 | `7a8c0a5fe49b_SKILL` | 93 | Q1 | document processing / insurance | CLI reference + tips | only Apache-2.0 file in the corpus (all others MIT or Unlicense); tests licence diversity |
| 4 | `541a1bc0d709_SKILL` | 173 | Q2 | software engineering / CI | imperative policy + decision table | by far the most rule-dense prompt in the corpus; the strongest possible case for the method, and the clearest demonstration that run-time guards do not lift |
| 5 | `23b1dabda63a_SKILL` | 176 | Q2 | healthcare analytics | staged workflow | safety-critical domain (medical disclaimer, evidence grading); heavy data-quality conditionals |
| 6 | `8ad9c110c8fc_SKILL` | 237 | Q2 | messaging / integrations | subcommand dispatcher | bilingual; the only file with two independent liftable environment branches; a 10-row routing table |
| 7 | `728fe4446723_SKILL` | 259 | Q3 | technical writing | format specification + persona | non-code output domain; contains an embedded `<!-- system-prompt -->` region and a 3-way section dispatch |
| 8 | `053136ecb86a_SKILL` | 302 | Q3 | security / incident response | procedure, Chinese-dominant | security domain with a dense safety-rule block; a 5-way ASCII flowchart router |
| 9 | `ab8bd009141a_SKILL` | 339 | Q3 | research / data visualisation | 9-phase pipeline | most conditionals of any file we could actually lift (3); heavy tool-invocation procedure |
| 10 | `b1950a5dde10_SKILL` | 515 | Q4 | finance / investing | persona + knowledge base, 59% Han | most Chinese-dominant file in the corpus; zero code blocks; the style/persona end of the distribution; explicit ranked priority lists |
| 11 | `9deacc979622_SKILL` | 741 | Q4 | software methodology | mode dispatcher + policy | 11-way `mode:` dispatch — the clearest example of a construct the grammar cannot express |
| 12 | `ccb5349ebdcc_SKILL` | 1388 | Q4 | SEO / content marketing | API reference + response guidelines | longest file in the corpus; ~85% endpoint documentation, which stresses the rule-extraction judgement at the low-density end (32 source lines per rule) |

Spread achieved: lengths 27–1,388 (mean 360, median 248); 3 files with substantial Chinese
content; 3 licences; 12 distinct application domains; 6 structural types (reference doc,
router, workflow, policy, persona, knowledge base).

Not chosen, and why: `6322785fcdfe` and `92e0713042d7` contain literal `{{` sequences in
code samples that the renderer would interpret as interpolations, which would have broken
the byte-fidelity guarantee; `bfdd75f608ff` (1,088 lines, crypto security) was passed over
in favour of `ccb5349ebdcc` to avoid a second security-domain file at the long end.

## 8. Per-file notes

Counts below: `rules` / `cond` = `// @condition` / `bp` = `// @breakpoint` /
`if` = `{% if %}` blocks / `else` = `{% else %}` arms / `ctx` = generated contexts.
"Dispatch" counts multi-way (3+) constructs observed and deliberately **not** lifted (C4).

### `09e319ab5af3_skill` — 27 src lines · 3 rules · 0 cond · 0 bp · 0 if · 0 else · 1 ctx

Weakest annotation in the set, and deliberately so. The file is an MCP tool contract:
description, parameters, returns, notes. It contains no imperative addressed to the agent.
The three rules are capability/contract statements (what the tool does, what it returns,
what it returns on failure) — the most generous honest reading. Line 23,
"If the city is not found or API fails, returns an error message", is an `If` that
describes the *tool's* behaviour, not a branch in the agent's instructions, so it is a flat
`format` rule.

Judgement call: one could argue for 0 rules here. We annotated 3 to avoid an empty file,
and flag that a stricter reading gives 0. Dispatch: 0.

### `194e3e6b9421_SKILL` — 73 · 11 · 0 · 0 · 0 · 0 · 1

A pipeline router. Six of the eleven rules are the routing bullets at lines 68–73.

Not lifted, and this is the most interesting negative result in the set:

- L70 "If this is the user's first time **and** no `strategy.md` exists" and L71
  "If `strategy.md` exists **but** no content" are guards on the *workspace at invocation*
  — they satisfy C1 and would be genuinely useful template branches — but both are
  conjunctions over two distinct properties, so they fail C3. The grammar simply cannot say
  this.
- L26–34, the `UPGRADE_AVAILABLE` / `JUST_UPGRADED` handling, branches on the output of the
  preamble bash the agent runs, so it fails C1. Note it is a three-level nested conditional
  in prose ("If Step 1 results in Yes or Always ... If Not now or Never ...").

Dispatch: 1 (the six-way routing list).

### `7a8c0a5fe49b_SKILL` — 93 · 7 · 0 · 0 · 0 · 0 · 1

Mostly CLI reference. `RULE:CATEGORY_SET` annotates the fixed classification vocabulary
(line 13) because that vocabulary is a real constraint on the agent's output, not
documentation. `RULE:PIPELINE_STAGES` covers the four-step Architecture list as one rule,
since the steps describe the tool's internal pipeline rather than four separate agent
obligations.

Not lifted: L93 "If a classification is wrong, use `reclass`" — the guard is the agent's
own assessment of its output (fails C1). Dispatch: 0.

### `541a1bc0d709_SKILL` — 173 · 70 · 0 · 2 · 0 · 0 · 1

The densest file in the corpus: 2.5 source lines per rule. Also the clearest demonstration
of finding #4.

- The eight-row Decision Table (L32–41) is annotated **per row** — the only table in the
  set treated this way — because each row is a complete "condition → action" rule the agent
  must apply, and rows 1–8 are the file's operative content. Priority 30 throughout,
  because the section is titled "Decision Table (**hard rules**)".
- Every guard in the file (`Any FAIL CI check is failing`, `pr.mergeable=dirty|behind`,
  `Reviews marked [STALE]`, `If bellwether is not found`) is resolved by the output of
  `bellwether check --watch` or a local command run mid-loop. All fail C1. **Zero branches.**
- Priority 30 is applied uniformly to the seven bullets of the section headed
  "## Critical: Use the bellwether CLI", including L25 which is itself a fallback
  ("If bellwether is not found, install it first"). Judgement call: the section title marks
  the whole block as overriding, so we did not down-rank the fallback within it.
- Two breakpoints, both from explicit gates: "DO NOT proceed to Phase 2 until CI is green"
  and "DO NOT start Step C until the commit exists and is pushed".

Dispatch: 1 (the eight-row table, which we annotated but did not lift).

### `23b1dabda63a_SKILL` — 176 · 28 · 0 · 1 · 0 · 0 · 1

Clinical report generator. Twelve `format` rules — the highest proportion in the set —
because most of its instructions constrain the shape of the produced report.

Not lifted:

- L62–65, the four-way data-sufficiency ladder (`insufficient` / `low` / `moderate` /
  `high`), fails C4. This is the single most "template-shaped" construct in the whole
  sample and the grammar still cannot express it.
- L143–156, report language. The guard (the user's language) *would* satisfy C1, but the
  source explicitly instructs the agent to **detect** it with a three-level priority rule,
  making it an agent-side determination; and L156 packs the Chinese-specific instruction
  into one line with a general one, failing C2. Left as two flat rules.
- L127 "If CGM data is absent, note that ..." — a guard embedded in a bullet that also
  states the general rule; fails C2.

`RULE:MEDICAL_DISCLAIMER` carries no priority: the source bolds it but never says it
overrides anything. Dispatch: 2.

### `8ad9c110c8fc_SKILL` — 237 · 48 · 2 · 1 · 2 · 2 · 3

The best case for the method in the whole sample: two independent, genuinely
context-resolvable, genuinely binary, genuinely block-scoped conditionals.

- `CONDITION:RUNTIME` (`{% if runtime.claude_code %}` / `{% else %}`, L57–58). The source
  presents exactly two numbered environments — Claude Code (where `AskUserQuestion` exists)
  and Codex/other (where it does not). Property of the environment; resolvable before
  render; binary; the source sets the two cases apart as list items under a dedicated
  "Runtime detection" heading.
- `CONDITION:CONFIG_PRESENT` (`{% if not config.exists %}` / `{% else %}`, L66–69). The
  source labels the two arms explicitly: "**If it does NOT exist:**" / "**If it exists:**".
  Property of the filesystem at invocation.

Not lifted:

- L36–45, the ten-row command-parsing table (user phrasing → subcommand), fails C4. One
  rule on the table's introducing line.
- L183–184, "If `CODEX_THREAD_ID` exists ... / Otherwise detect the current Claude Code
  session ..." is binary and environment-resolvable, but it sits as two bullets inside an
  eleven-bullet "Behavior:" list that the source does not set apart — fails C2. Annotated
  as two rules with `priority=10` on the fallback. This is the closest call in the set.
- L91 "For each enabled channel, collect one credential at a time" — the channel set is
  chosen during the wizard the agent runs, so this is not a template loop.

Dispatch: 1.

### `728fe4446723_SKILL` — 259 · 29 · 1 · 0 · 1 · 0 · 2

Man-page generator; the non-code output domain in the sample.

- `CONDITION:MANSPLAIN_CLI` (`{% if env.mansplain_cli %}`, L38–49) guards the whole
  "## Alternative: maintain man pages in markdown" section. The source sets it apart with
  its own heading and opens it with "If the `mansplain` CLI is installed". Environment
  property, single predicate, block-scoped, no else arm — the source offers no
  "otherwise" text, so we did not fabricate one.

Not lifted:

- L34 "Validate with `mandoc -Tlint <file>` if mandoc is available" and L35 "If `mansplain`
  CLI is installed, use `mansplain lint <file>`" are the same kind of guard but attached to
  single steps inside a numbered procedure — fails C2. Wrapping them would also have
  produced a numbered list rendering as 1,2,3,5,6.
- L201–214, the three-way section-1/5/7 guidance ("For section 5 (file formats): ..."),
  fails C4. This is exactly the "for Python files ..." shape the method targets, and the
  grammar cannot express it because there are three cases.

Priorities: `RULE:RONN_ALTERNATIVE` gets 10 and `RULE:PREFER_MDOC_FOR_INITIAL` gets 20,
from the source's own "For initial generation, writing mdoc directly ... is preferred".
Dispatch: 1.

### `053136ecb86a_SKILL` — 302 · 22 · 0 · 0 · 0 · 0 · 1

Security incident-response toolkit, Chinese-dominant. Eleven of 22 rules are `safety` — the
highest safety proportion in the set — covering API-key handling, SSH credential hygiene,
and the five-item "安全注意事项" block (authorised use only, isolated environment, protect
results, legal compliance, no destructive actions on production).

Priority 30 on those eleven is justified by the source's own markers: "## ⚠️ 重要：配置必读"
and "> ⚠️ **重要提示**".

Not lifted: the "智能路由逻辑 / 判断流程" ASCII flowchart (L133–147) is a five-way
first-match router ending in "无法判断 → 询问用户选择模块". Fails C4. Annotated as one rule
for the flow plus `RULE:ASK_WHEN_AMBIGUOUS priority=10` for the fallback arm. Dispatch: 1.

### `ab8bd009141a_SKILL` — 339 · 49 · 3 · 3 · 3 · 2 · 4

The most branch-rich file in the sample (3 of the 9 conditions).

- `CONDITION:NO_TOPIC` (`{% if not invocation.topic %}`, L55–57) guards the whole
  "### No-Topic Invocation" section. Whether a topic was supplied is the definition of an
  invocation-context property. No else arm in the source.
- `CONDITION:HUB_CONFIG` (`{% if hub.config_exists %}` / `{% else %}`, L68–73). The source
  writes list items 2 and 3 as "**If exists:** ..." / "**If not exists → First-Time
  Setup**" — an explicit binary pair on filesystem state.
- `CONDITION:PHASE_TIMING` (`{% if not timing.enabled %}` / `{% else %}`, L91–105). The
  source labels the arms "**When disabled:**" and "**When enabled:**". The negated form is
  used because the source presents the disabled case first and we preserve line order.

Not lifted: the extension-detection conditional (L124) depends on a scan the agent
performs; the visibility rules at L182 depend on `confirmEachShare` but are stated as one
compound sentence (fails C2 and C3). Three breakpoints, all from explicit halts
("halt with ...", "ALWAYS wait for explicit user response", "Steps 1-5 must complete
before 6-9"). Dispatch: 0.

### `b1950a5dde10_SKILL` — 515 · 67 · 1 · 0 · 1 · 0 · 2

Persona/knowledge-base prompt, 59% Han characters, zero code blocks. Eighteen `style` rules
— the style end of the distribution.

- `CONDITION:COMPANY_ANALYSIS` (`{% if question.type == "company" %}`, L44–77) guards the
  entire "### Step 2: 巴菲特式研究（仅当分析具体公司时）" section. The parenthetical
  "**only when** analysing a specific company" is the source's own guard, the section is its
  own block, and the guard is a property of the user's question. No else arm — Step 3
  follows unconditionally.

Judgement calls:

- The eight rules under "## 角色扮演规则（最重要）" all carry `priority=30`, because the
  heading declares the section "most important".
- "我追求的（按优先级）" ranks five values explicitly, mapped to 50/40/30/20/10 per §4. This
  is the only file using values above 30 other than `9deacc979622`.
- "数据处理策略" ranks three data sources explicitly (优先级1/2/3) → 30/20/10.

Not lifted: the 3-way question-type table (L38–42) and the 3-way data-priority ladder, both
C4. Inside the lifted section, "2. **如果用户没有，使用WebSearch**" is a further conditional
but it depends on the user's answer to a question the agent has just asked — fails C1.
Dispatch: 2.

### `9deacc979622_SKILL` — 741 · 78 · 1 · 4 · 1 · 0 · 2

Software-methodology skill; the clearest example of grammar limits.

- `CONDITION:BEGINNER` (`{% if user.beginner %}`, L79–100) guards the whole
  "### 1. Start with a plain-language explanation when needed" section.
  **Flagged judgement call (C3):** the source's guard is a disjunction — "if the user is a
  beginner, non-programmer, or seems unsure what the process means". We modelled it as one
  boolean path because the three disjuncts enumerate values of a single underlying property
  (the user's familiarity), and the section heading itself abstracts them to "when needed".
  A stricter reading would refuse this lift and leave the file with zero branches.

Not lifted:

- L434–708: an **eleven-way** `mode:` dispatch (`explain_plainly`, `initialize_project_docs`,
  `initialize_coding_project`, `git_checkpoint`, `discover_urd`, `analyze_add`, `design_mdd`,
  `write_tdd`, `plan_rmd`, `compile_wiki`, `update_docs`). The mode *is* an invocation-context
  property and each mode block *is* block-scoped — this is a textbook template dispatch — and
  C4 blocks it because the grammar has no `elif`. Annotated as one `process` rule per mode.
- L239–244, a four-way stack-defaults ladder (C4), annotated with ranked priorities 40/30/20/10.
- L391–433, the three-way document-strength ladder (simple/standard/strict) (C4).
- L258 "Use `uv init` when uv is available. If uv is unavailable, create the same essential
  files ..." — a perfectly good binary environment guard, defeated by C2: both arms are on
  one source line, and splitting them would edit the text.
- L281 "... must end with a Git checkpoint **unless** the user explicitly disables git" —
  same problem, clause-level.

Four breakpoints, all from explicit gates ("may move to Design Split **only when** all of
these are present", "Ask for explicit permission before the first push", "stop before
inventing behavior", "Before handing off project docs, verify"). Dispatch: 3.

### `ccb5349ebdcc_SKILL` — 1388 · 43 · 1 · 2 · 1 · 0 · 2

Longest file in the corpus and the lowest rule density: 32 source lines per rule, against
2.5 for `541a1bc0d709`. Roughly 85% of it is HTTP endpoint reference — request shapes,
parameter lists, credit-pricing tables, webhook event tables — which contains no agent
obligations. The behavioural content is concentrated in three places: the setup flow, the
"Limitations" list, and the eleven "Response Guidelines" bullets.

- `CONDITION:API_KEY_MISSING` (`{% if not agent.api_key %}`, L84–139) guards the entire
  "### Setup (run once)" flow. The source's own sentence is "If you don't have a saved API
  key for Citedy, run this flow:". Credential presence is an environment property; the
  guarded region is four `####` sub-sections; binary; no else arm in the source.

Not lifted: the five-row HTTP error-handling table (401/402/403/429/500) is a dispatch on a
value produced at run time (fails both C1 and C4); the seven-row "Choosing the Right Path"
table is a dispatch on user intent (C4) — one rule on the section heading.

Priorities: `REGISTER_SCRIPT` 20 / `REGISTER_API_DIRECT` 10, from the source's own
"**Preferred:**" / "**Alternative:**". Dispatch: 2.

---

## 9. Coverage results

Contexts were generated deterministically with `promptdbg gen-contexts` and coverage
measured with the `annotation` engine. All 12 files parse, render, and produce coverage.

| file | src lines | rules | cond | bp | if | else | ctx | rule cov (struct) | rule cov (trace) | cond cov | branch-outcome cov | rule-pair cov |
|---|---:|---:|---:|---:|---:|---:|---:|---|---|---|---|---|
| `09e319ab5af3_skill` | 27 | 3 | 0 | 0 | 0 | 0 | 1 | 3/3 (100%) | 3/3 (100%) | 0/0 (100%) | 0/0 (100%) | 3/3 (100%) |
| `194e3e6b9421_SKILL` | 73 | 11 | 0 | 0 | 0 | 0 | 1 | 11/11 (100%) | 11/11 (100%) | 0/0 (100%) | 0/0 (100%) | 55/55 (100%) |
| `7a8c0a5fe49b_SKILL` | 93 | 7 | 0 | 0 | 0 | 0 | 1 | 7/7 (100%) | 7/7 (100%) | 0/0 (100%) | 0/0 (100%) | 21/21 (100%) |
| `541a1bc0d709_SKILL` | 173 | 70 | 0 | 2 | 0 | 0 | 1 | 70/70 (100%) | 70/70 (100%) | 0/0 (100%) | 0/0 (100%) | 2415/2415 (100%) |
| `23b1dabda63a_SKILL` | 176 | 28 | 0 | 1 | 0 | 0 | 1 | 28/28 (100%) | 28/28 (100%) | 0/0 (100%) | 0/0 (100%) | 378/378 (100%) |
| `8ad9c110c8fc_SKILL` | 237 | 48 | 2 | 1 | 2 | 2 | 3 | 48/48 (100%) | 48/48 (100%) | 2/2 (100%) | 4/4 (100%) | 1123/1128 (99.6%) |
| `728fe4446723_SKILL` | 259 | 29 | 1 | 0 | 1 | 0 | 2 | 29/29 (100%) | 29/29 (100%) | 1/1 (100%) | 2/2 (100%) | 406/406 (100%) |
| `053136ecb86a_SKILL` | 302 | 22 | 0 | 0 | 0 | 0 | 1 | 22/22 (100%) | 22/22 (100%) | 0/0 (100%) | 0/0 (100%) | 231/231 (100%) |
| `ab8bd009141a_SKILL` | 339 | 49 | 3 | 3 | 3 | 2 | 4 | 49/49 (100%) | 49/49 (100%) | 3/3 (100%) | 6/6 (100%) | 1168/1176 (99.3%) |
| `b1950a5dde10_SKILL` | 515 | 67 | 1 | 0 | 1 | 0 | 2 | 67/67 (100%) | 67/67 (100%) | 1/1 (100%) | 2/2 (100%) | 2211/2211 (100%) |
| `9deacc979622_SKILL` | 741 | 78 | 1 | 4 | 1 | 0 | 2 | 78/78 (100%) | 78/78 (100%) | 1/1 (100%) | 2/2 (100%) | 3003/3003 (100%) |
| `ccb5349ebdcc_SKILL` | 1388 | 43 | 1 | 2 | 1 | 0 | 2 | 43/43 (100%) | 43/43 (100%) | 1/1 (100%) | 2/2 (100%) | 903/903 (100%) |
| **total** | **4323** | **455** | **9** | **13** | **9** | **4** | **21** | **455/455** | **455/455** | **9/9** | **16/16** | **11917/11930 (99.9%)** |

Reading these numbers honestly: the 100% figures are **not** evidence that the method works
well on real prompts. They are a consequence of the structural flatness reported in §1 —
with no branches, one context reaches everything, and rule coverage degenerates to "the
file rendered". The only non-trivial numbers in the table are the branch-outcome columns
for the five files that have branches, and the two rule-pair figures below 100%, which come
from rule pairs that live in mutually exclusive arms and therefore can never co-occur.

## 10. Reproduction

The annotated files are generated from the sources by `apply_annotations.py` in this
directory, which holds the complete annotation spec as line-addressed insertions and
asserts the byte-fidelity property on every build:

```bash
python3 corpus/github-annotated/apply_annotations.py            # rebuild + per-file counts
python3 corpus/github-annotated/apply_annotations.py --review   # every insertion next to
                                                                # the line it annotates
```

To re-measure:

```bash
PD=tool/target/release/promptdbg
cargo build --release --manifest-path tool/Cargo.toml

for f in corpus/github-annotated/*.prompt.rtpl; do
  stem=$(basename "$f" .prompt.rtpl)
  $PD analyze "$f" --engine annotation --no-lint --format json
  $PD gen-contexts "$f" --out "corpus/github-annotated/$stem.contexts.json"
  $PD coverage "$f" --contexts "corpus/github-annotated/$stem.contexts.json"
done
```

`analyze --engine annotation` reports no warnings for any of the 12 (no duplicate node ids,
no dangling `conditionRef`). Context generation is deterministic, so the committed
`*.contexts.json` files are reproducible byte-for-byte.
