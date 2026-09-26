---
name: taiwan-stock-analysis-agent
description: Analyze Taiwan stocks using TWSE OpenAPI, TPEX/MOPS, Yahoo Finance, web search/news, and contrarian sentiment signals to produce structured research reports.
version: 1.0.0
author: Hermes Agent
license: MIT
metadata:
  hermes:
    tags: [Taiwan, stocks, TWSE, Yahoo Finance, analysis, news, sentiment, contrarian, research]
---

# Taiwan Stock Analysis Agent

## Purpose
Use this skill when the user wants analysis of Taiwanese equities, the Taiwan market, a watchlist, a sector, or a single stock.

This skill is designed to combine:
- **TWSE OpenAPI** for official market data
- **TPEX / MOPS** for OTC and financial disclosure data
- **Yahoo Finance** for analyst estimates, targets, peer context, and headlines
- **Web search / news crawling** for fresh catalysts and narrative shifts
- **Contrarian sentiment signals** (e.g. Banini-style reverse-indicator analysis) as an auxiliary risk layer

## Repository Status

This repository is documentation-first. The Python files in `scripts/` are placeholders for future data integrations, so the skill should be written to work even when some sources are unavailable.

## What this skill should produce
Return analysis in a way that is:
- **source-aware**: distinguish official data, analyst estimates, news, and sentiment
- **structured**: give a clear summary, evidence, risks, and conclusion
- **practical**: avoid vague commentary; tie claims to observed data
- **uncertain when appropriate**: if sources conflict, say so explicitly

## Core principles
1. **Official data is the anchor**
   - Prefer TWSE/MOPS/TPEX for facts about price, volume, disclosures, financials, and market structure.
2. **Yahoo Finance is a narrative and expectation layer**
   - Use analyst estimates, price targets, and related news to understand consensus and revision trends.
3. **News is catalyst context, not truth by default**
   - Verify important claims against official filings when possible.
4. **Contrarian sentiment is auxiliary**
   - Reverse-indicator / hype / panic signals can highlight crowded positioning, but must never override hard data.
5. **Separate signal from interpretation**
   - Report what was observed first, then what it may imply.

## Recommended workflow

### 1) Normalize the request
Identify:
- ticker(s)
- market scope: listed / OTC / sector / index
- time horizon: intraday / 1D / weekly / medium-term
- output type: quick take / full report / compare watchlist / event analysis

If the user gives a Taiwan ticker, normalize common formats like:
- `2330`
- `2330.TW`
- `2330.TWO`
- company name aliases

### 2) Collect official data first
Prefer these data types where available:
- price, change, open/high/low/close
- volume vs average volume
- P/E, P/B, dividend yield
- margin trading / short selling
- major announcements / disclosures
- corporate governance / penalty notices
- financial statements and revenue trends

### 3) Mid-session / 11:30 market checks: use official daily proxies when live intraday is limited
For a mid-session Taiwan market note, the best available live signal is often **not** a direct 11:30 tape snapshot. If intraday feeds are unavailable or partial, use TWSE official historical / daily pages as a proxy and state the limitation clearly.

Recommended fallback order:
1. **TWSE Highlights of Daily Trading** (`fmtqik`) to compare today’s total trade value / volume / index vs previous sessions.
2. **TWSE Top 20 by Volume** (`mi-stock20`) to identify where the day’s real liquidity is concentrating.
3. **Yahoo Finance quote pages** for narrative context, valuation, and whether a bellwether like TSMC is near earnings / guidance.
4. **Reuters / fresh web search** to validate whether the headline tape is already saturated with the same bullish/bearish theme.

When doing a mid-session read, explicitly compare:
- today’s trade value vs yesterday
- today’s volume vs yesterday
- whether index is up while liquidity is down
- whether leaders are still expanding or starting to stall

Interpretation rules:
- **Price up + volume down** = rising caution; rally may be losing sponsorship.
- **Strong leaders but ETF-heavy turnover** = sentiment can be hot but fragile.
- **Broad index strength with fading top names** = possible rotation / late-cycle feel.
- **Repeated bullish headlines around the same bellwether** = contrarian risk signal, not confirmation.

### 4) Add market narrative data
From Yahoo Finance, extract:
- current quote context
- analyst estimate tables
- revenue and EPS forecast trends
- target price range
- recent upgrades/downgrades if available
- peer comparison context
- headline/news themes

### 5) Add fresh news and event context
Use web search or crawling to gather:
- company announcements
- earnings / guidance / investor presentation
- macro or sector news
- supply chain / policy / regulatory catalysts
- repeated headlines that indicate a theme

### 6) Add contrarian sentiment
Treat hype/panic/repetition as a **risk modifier**.
Useful signals include:
- extremely bullish language with no new facts
- crowded consensus
- retail FOMO or panic language
- market overreaction relative to fundamentals

If a separate Banini / reverse-indicator skill exists in the workspace, use it as an **optional sentiment adapter** rather than a core dependency. The core agent should still work without it by using general contrarian heuristics from news, search results, and market crowding signals.

### 7) Synthesize into a scorecard
When enough data exists, provide a scorecard with:
- **Trend**: bullish / neutral / bearish
- **Fundamentals**: improving / stable / weakening
- **Valuation**: cheap / fair / rich / unclear
- **Catalysts**: positive / neutral / negative
- **Sentiment**: crowded / balanced / fearful / euphoric
- **Risk**: low / medium / high
- **Confidence**: low / medium / high

If signals conflict, prefer “uncertain” over forced certainty.

## Output Expectations

When answering the user, keep the result compact and evidence-led:
- start with the most important conclusion
- cite the sources used or clearly name them in prose
- separate raw observations from interpretation
- mention missing data explicitly instead of filling gaps with guesses

## Minimal Implementation Guidance

If you later wire this skill into code, keep these boundaries:
- fetch official market data first
- enrich with Yahoo Finance only after the official view is collected
- use news as event context, not as final proof
- treat contrarian signals as a warning layer only

## Output format
Use this default structure unless the user asks otherwise.

### For a single stock
1. **Snapshot**
   - ticker, company name, market, price context
2. **What the market is saying**
   - analyst targets, revisions, consensus, related headlines
3. **What the official data says**
   - price action, volume, valuation, disclosures, financials
4. **Catalysts and risks**
   - upcoming earnings, guidance, news, sector conditions
5. **Contrarian view**
   - possible overheat / panic / crowded trade warning
6. **Conclusion**
   - concise verdict with caveats

### For a watchlist or comparison
Provide a table with:
- ticker
- price trend
- valuation
- earnings momentum
- news intensity
- sentiment score
- risk rating
- final note

### For market / sector analysis
Provide:
- macro context
- sector strength/weakness
- breadth / concentration
- leading names
- key risks
- conclusion

## Suggested language for conclusions
Use cautious, decision-support wording such as:
- “偏多，但估值已偏高，追價風險上升。”
- “基本面支撐仍在，但短線情緒過熱。”
- “消息面偏正向，但需等待財報 / 指引確認。”
- “反指標訊號升溫，應提高風險意識。”

Avoid absolute buy/sell language unless the user explicitly asks for a trading-style opinion.

## Important caveats
- Do not claim real-time certainty unless the underlying data was fetched successfully.
- If Yahoo pages fail or partially render, say so and fall back to whatever visible data was available.
- If news is duplicated across many outlets, mention that it is syndicated rather than treating it as independent confirmation.
- If only partial data is available, clearly label the report as partial.

## Best practices
- Prefer **facts first, interpretation second**.
- Cross-check important claims across at least two sources when possible.
- Treat analyst estimates as expectations, not facts.
- Treat contrarian signals as a warning layer, not a standalone decision rule.
- Keep the report readable: concise bullets plus a short verdict.

## Example use cases
- “分析 2330.TW 今天為什麼漲”
- “幫我看 2317.TW 的基本面、籌碼和新聞”
- “比較 2330.TW 與 2454.TW 哪個更有機會”
- “做一份台股盤前重點摘要”
- “用反指標角度看目前市場是否過熱”

## Maintenance Notes

If the repository grows, keep the following files aligned:
- `README.md` and `README_zh.md`
- `references/data-sources.md`
- `references/output-schema.md`
- `references/scoring-rules.md`

Update them together whenever source priority, schema fields, or scoring labels change.

## If you need to extend this skill later
Future extensions can add:
- a dedicated cache layer
- formal JSON output schema
- sector rotation logic
- backtesting hooks
- alerting / cron support
- a separate OTC or small-cap mode
