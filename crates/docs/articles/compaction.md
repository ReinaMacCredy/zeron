Every agent has a context window, the amount of conversation it can hold at once. Compaction replaces the older conversation with a summary so the chat can keep going and later turns stay cheaper. A chat compacts for one of three reasons, and each has one decider:

| Reason | Who decides | When |
| --- | --- | --- |
| The prompt cache is about to expire | Zeron | An idle chat, before its 1-hour cache runs out |
| The work changed | The agent | It calls its `compact` tool; the compaction runs when the turn ends |
| The window is full | Claude Code | On its own, near the window, and the turn carries on |

You can also compact by hand at any time.

## See how full the context is

The ring at the right of the session footer shows how much of the context window is in use. It turns to a warning color at 75% and to danger at 90%. Click it to open **Context window**, with the tokens used and remaining.

The bar in that card carries up to three marks. Hover one to read it:

- where the agent starts being reminded that a new task should compact first
- where the agent is warned that the window is nearly full, at 85% of the next mark
- where Claude Code compacts on its own

A mark is left out when it does not apply: the reminder is **Off**, the reminder point is past the warning, or the agent has no known point where it compacts by itself.

![The context window popover](compaction-popover.jpg)

## Next compaction

The same card has a **Next compaction** section. It always says one thing:

| It reads | Meaning |
| --- | --- |
| **Idle** and a time | Zeron compacts then if the chat stays idle. A message from you first cancels it. |
| **Agent asked · when this turn ends** | The agent called `compact`. A message from you first cancels it. |
| **Claude Code** and a size | The context is past 85% of the point where Claude Code compacts by itself. |
| **Compacting now** | A compaction is running. |
| **Not scheduled** and a reason | Nothing is planned, and why. |

The reasons under **Not scheduled**: the agent is working; idle compaction is off in Settings; mission threads are excluded in Settings; the agent reports no prompt cache, or only the 5-minute one; the context is under the size set in Settings; the chat already compacted since your last message; the prompt cache already expired; or background work is running on an agent that would have to restart to compact. When no reason is on record, such as a chat that has not run a turn since Zeron started, it reads **Not scheduled** alone.

When a compaction is planned, a short note sits beside the ring in the footer, such as **compacts 3:42 PM** or **compacts after this turn**, so you can tell without opening the card.

## Compact by hand

Click **Compact now** in that card, or type `/compact` in the composer. You can add a focus for the summary:

```
/compact keep the API design decisions
```

Claude Code, Codex, OpenCode, Pi, and ACP agents that offer `/compact` can compact. While a turn or a compaction is running, compact by hand is not offered. An agent other than Claude Code restarts its process to compact, so while background work it started is still running, compaction is refused rather than ending that work.

When you open a Claude Code chat of 100K tokens or more that has been quiet for 70 minutes or longer, its prompt cache has expired. A **Resume with less context** card offers **Compact**, or **Keep full history** to send as is.

## The cache is about to expire

Claude keeps a chat's prompt cache for 1 hour after it was last used. Once it expires, your next message makes Claude reprocess the whole chat. With this row on, Zeron summarizes a long idle chat 54 minutes after the cache was last used, at 90% of its life, so coming back reprocesses the summary instead.

A chat qualifies when all of these hold:

- It is a Claude Code chat whose last turn wrote to the 1-hour cache. Other agents do not report a cache lifetime, so they are never compacted while idle.
- Its context is at least **Only chats larger than**.
- No turn is running. Background shells and subagents do not hold a Claude Code chat back, because Claude Code compacts in place and they keep running.

The summary is the agent's own `/compact` with no focus. Sending a message first cancels it. If the Mac was asleep at the planned moment, Zeron still compacts when it wakes, as long as the cache has a minute or more left. After the engine restarts, idle chats used in the last hour are scheduled again. Each cache use gets at most one idle compaction.

## The work changed

This row gives agents Zeron's `compact` tool. It is the same switch as **compact** in [Settings > Context & tools > Tools](zeron://settings/tools).

An agent calls the tool when a piece of work is done and what comes next needs only its conclusions. It can name what the summary should keep, and it can write a short note, such as "Continue with slice 3", that is sent back to it as its next message once the compaction is done, so the work goes on without you.

The compaction never runs inside the turn that asks for it. It runs as soon as the turn ends, as the agent asked: no other model reviews the request, and nothing is carried over besides the summary and the agent's note. Any message that reaches the agent first cancels it, and so does **Stop**; the agent is told. Agents that restart their process to compact refuse while background work they started is still running, and ask the agent to try again later.

Two reminders help the agent decide. Both state the context in tokens and as a share of the window:

- **On your message.** Past **Remind it from**, each message you send tells the agent: if this starts a new task or topic, call compact first with the message as the note, and do nothing else this turn. A follow-up is never a reason to compact.
- **During a long turn.** At 85% of the point where Claude Code compacts by itself, the agent is told once, and again for every 50K tokens the context grows. It may compact at a clean break, or keep going.

**Remind it from** counts only the context a compaction could free: what the chat holds above what was left after its last compaction. If 70% of what the chat's window leaves is lower than the setting, the reminder starts at that 70% instead. **Off** turns the reminder on your message off.

## The window is full

Claude Code compacts on its own when the context reaches its window minus 33K, and keeps working on the turn. The window is the one set by the **Context** card in the model picker. Zeron does not start this compaction and has no switch for it.

## Mission threads

**Mission threads** decides whether a chat with a plan and its step threads are compacted automatically. On (the default), they follow the same rules as any chat. Off leaves them alone: no idle compaction and no reminders. A chat stays a lead after every step of its plan is done. The agent's own compact tool still follows **The work changed**, and you can still compact by hand. See [Mission](zeron://docs/mission) for plans.

## What you see in the transcript

A compaction is one divider that follows it through: **Compaction queued · starts when the turn settles**, then **Compacting** with the time so far, then how it ended, with the context before and after and how long it took:

| Divider | Cause |
| --- | --- |
| **Context compacted** | You compacted by hand. |
| **Compacted while idle, before the prompt cache expired** | Idle compaction. |
| **Agent compacted** | The agent's compact tool. |
| **Compacted, window full** | The agent's CLI compacted on its own. |

Click a finished divider to open it. **Asked to keep** is the focus the agent or you gave, when there was one. **Summary** is what the agent wrote.

A message you send while a compaction runs waits in the queue, marked **Sends after compaction**, and goes out when it ends.

Chats from earlier versions may hold classifier decisions. They are no longer shown; the compactions around them appear as plain dividers.

## Settings

In [Settings > Agents > Compaction](zeron://settings/compaction):

| Row | Default |
| --- | --- |
| **The cache is about to expire** | On |
| **Only chats larger than** (100K, 150K, 200K) | 100K |
| **The work changed** | On |
| **Remind it from** (Off, 100K, 150K, 200K) | 150K |
| **Mission threads** | On |

## Turn it all off

Switch off **The cache is about to expire** and **The work changed**. Zeron then compacts a chat only when you ask. Claude Code still compacts by itself when its window fills; the **Context** card in the model picker sets where.

## Long context for Claude

For Claude models that support it, the model picker has a **Context** card. **Long context** switches between a 1M and a 200K window, and its slider sets when the chat compacts. Either change starts a new Claude process on your next message.
