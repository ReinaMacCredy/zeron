The Usage page shows how much of each agent plan you have left, and what your chats on this device would cost at API prices.

![The Usage page](usage.jpg)

## Open Usage

Click **Usage** in the rail, pick **Usage** in the account menu, or pick **Open usage** in the Cmd+K palette. The plan rings in the rail open a card with an **Open Usage** row, too.

## Plan limits

**Plan limits** has a card for the active account of each agent that reports usage. Each limit, such as **5h** or **Weekly**, shows the percent left and when it resets. The bar turns to a warning color as you near the limit.

Under each bar, Zeron compares your pace with the time left: **in reserve** or **in deficit**, and **Lasts until reset**, **Runs out in** a time, or **Limit reached**. Limits refresh after each turn. Open **Details** for the account and last update time.

## Token usage

**Token usage** reads the Claude Code, Codex, and Grok transcripts on this device and adds them up.

- Switch between **Cost** and **Tokens**.
- Choose **Past 24h**, **7 days**, **30 days**, or **90 days**.
- Click the refresh button (**Refresh usage**) to read the transcripts again.

The chart shows one line per agent. **Totals** lists **Processed tokens**, **Cached input**, **Uncached input**, **Output**, and **Cache savings**. **Breakdown** splits the spend by **Model** or by **Day** (by **Hour** for Past 24h).

Cost is an API estimate. Zeron uses the cost a transcript reports when there is one, and otherwise prices the tokens at public API rates from models.dev. Responses on a model with no known price are left out, and the page says how many.

## Session stats in the footer

In a git project, the session footer shows a chip such as **86 turns 1903 steps · 137 tok/s**. For Claude Code chats it adds the prompt cache's time left, or **cold** once it has expired.

Hover the chip for **Session cost** and **Session statistics**. The cost uses the total Claude Code reports, or an estimate from public API prices, marked **≈**. The statistics include **LLM time**, **Tool time**, **Avg time to first token (TTFT)**, **Tokens per second (TPS)**, **Cache TTL**, and **Cache hit rate**.

While an agent works, the working row in the transcript also shows its live speed, such as **· 42 tok/s**.

> To pick up a thread that hit a usage limit, turn on **Auto-resume after usage limits** in [Settings > Providers](zeron://settings/providers).
