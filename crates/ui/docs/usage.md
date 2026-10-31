The Usage page shows how much of each agent plan you have left, and what your chats on this device would cost at API prices.

![The Usage page](usage.jpg)

## Open Usage

Click the usage ring in the rail, type `/usage` in a Claude Code chat, or pick **Open usage** in the Cmd+K palette.

The ring shows the tightest limit window of the agent accounts you use. Hover it for a card with each account's windows, how much of each is used and when the binding one resets; the card is for reading only. At a limit the ring turns red with a **!**.

## Plan limits

**Plan limits** has a card for the active account of each agent that reports usage. Each limit, such as **5h** or **Weekly**, shows the percent left and when it resets. The bar turns to a warning color as you near the limit.

Under each bar, Zeron compares your pace with the time left: **in reserve** or **in deficit**, and **Lasts until reset**, **Runs out in** a time, or **Limit reached**. Limits refresh after each turn. Open **Details** for the account and last update time. When a provider has more than one account, click the account name on its card to pick another. [Routing](zeron://docs/routing) reads the same limits to move a delegated child off an agent that is out of quota.

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

## At a limit

When a chat's agent reaches a limit, one notice in that chat's composer names the limit and when it resets, with a button to switch to another model and **Wait for reset**. While Zeron waits for the reset, the notice says so and offers **Cancel**.

> To pick up a thread that hit a usage limit, turn on **Auto-resume after usage limits** in [Settings > Providers](zeron://settings/providers).
