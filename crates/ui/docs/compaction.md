Every agent has a context window, the amount of conversation it can hold at once. Compaction replaces the older conversation with a summary so the chat can keep going and later turns stay cheaper. Zeron can compact a chat in three ways: when it sits idle, when the agent asks, and when your next message starts a new task. You can also compact by hand at any time.

## See how full the context is

The ring at the right of the session footer shows how much of the context window is in use. It turns to a warning color at 75% and to danger at 90%. Click it to open **Context window**, with the tokens used and remaining. When a compaction is planned, the popover also says when: **Compacts at** a time for idle compaction, or **Compacts when this turn ends** when the agent asked.

![The context window popover](compaction-popover.jpg)

## Compact by hand

Click **Compact context** in that popover, or type `/compact` in the composer. You can add a focus for the summary:

```
/compact keep the API design decisions
```

Claude Code, Codex, OpenCode, Pi, and ACP agents that offer `/compact` can compact. Otherwise the button is dimmed and says why: **Compaction in progress**, **Wait for the current turn to finish**, or that the agent does not support compaction. An agent other than Claude Code restarts its process to compact, so while background work it started is still running, compaction is refused rather than ending that work.

When you open a Claude Code chat of 100K tokens or more that has been quiet for 70 minutes or longer, its prompt cache has expired. A **Resume with less context** card offers **Compact**, or **Keep full history** to send as is.

## Idle compaction

Claude keeps a chat's prompt cache for 1 hour after it was last used. Once it expires, your next message makes Claude reprocess the whole chat. With **Compact idle chats before the cache expires** on, Zeron summarizes a long idle chat 54 minutes after the cache was last used, at 90% of its life, so coming back reprocesses the summary instead.

A chat qualifies when all of these hold:

- It is a Claude Code chat whose last turn wrote to the 1-hour cache. Other agents do not report a cache lifetime, so they are never compacted while idle.
- Its context is at least the **Idle threshold**.
- It is idle: no turn running, no subagents, and no background tasks.

The summary is the agent's own `/compact` with no focus. Sending a message first cancels it. If the Mac was asleep and the moment passed more than a minute ago, Zeron skips it rather than compacting late. Each cache use gets at most one idle compaction.

## Let agents compact themselves

This switch gives agents Zeron's `compact` tool. It is the same switch as **compact** in [Settings > Context & tools > Tools](zeron://settings/tools).

An agent calls the tool at a natural break, for example when a piece of work is done and verified. It can name what the summary should keep, and it can write a short note, such as "Continue with slice 3", that is sent back to it as its next message once the compaction is done, so the work goes on without you.

The compaction never runs inside the turn that asks for it. The transcript shows **Compaction queued · starts when the turn settles**, and it runs as soon as the turn ends. Any message that reaches the agent first cancels it, and so does **Stop**; the agent is told. Agents that restart their process to compact refuse while background work they started is still running, and ask the agent to try again later.

In Reminder mode, every message past **Remind at** (see below) reminds the agent to consider compacting. In Claude Code chats past that point, a long run of tool calls also gets a reminder for every 50K tokens the context grows, in either mode.

When the agent wrote a next-message note and **Compact new tasks** is **Classifier**, the note is checked before the compaction runs:

- With **Agent model**, the classifier reads the note against the recent conversation. It can carry the last turns over or decline. It never compacts to the summary alone. When it declines, the agent is told why and keeps working.
- With Jev, or when no agent can run the check, the note is not sent anywhere: the chat compacts and carries the last two turns over.
- When the context is at 85% of the point where the agent's CLI compacts by itself (for Claude, the window minus 33K), the chat compacts and carries the last turns over without asking.

Without a note, or in Reminder mode, the compaction runs as the agent asked.

## Compact new tasks

This row decides how a chat notices that your message starts a new task.

- **Reminder**: each of your messages carries a reminder, and the agent decides. If the message starts a new task, it calls compact first with your message as its note and does nothing else that turn. Reminders need **Let agents compact themselves** on.
- **Classifier** (the default): before your message reaches the agent, a separate model reads the last few turns and your message, and decides. This works even with **Let agents compact themselves** off.

The classifier has three answers:

| Answer | What happens |
| --- | --- |
| **New task** | The chat compacts to a summary of what your message may need, then your message is sent. |
| **Carry over** | The chat compacts, and the last two turns go along word for word ahead of your message. |
| **Same task** | Nothing changes. |

A new task needs clear evidence: you name a fresh start or a different task, or ask for a different deliverable right after the agent reported the work finished. Follow-ups, approvals ("ok, do it"), asking to test, merge, or ship what was just done, and reports about it are never a new task. When unsure, it carries over. A message that refers back to something older than the last two turns, or arrives while a build, test, or subagent is still running, stays **Same task**. So does a message that only asks to commit, push, merge, open a PR for, or release the work the agent just reported done ("commit đi"): that closing step runs at once, and the chat compacts at your next message instead.

The carried turns are your message and the agent's final reply for each of the last two turns, up to 8,000 characters. Tool calls, their output, and thinking are left out. Background shells, subagents, and delegated tasks still running are listed after them, so the agent knows to expect their results.

With **Agent model**, a **Same task** answer gets a second look that can only change it to **Carry over**. After three **Same task** answers in a row while the context keeps growing, the next check carries over without asking. If the classifier gives no answer, the chat is not compacted, and it tries again on a later message.

For an agent that cannot compact, a **New task** or **Carry over** answer starts a fresh session instead. The earlier conversation goes along as text, the most recent 24,000 characters of it; tool results and file contents do not.

## Compact ahead of time (Experimental)

**Compact ahead of time** sits under **Compact new tasks** and is off by default. It moves the compaction off the moment you send: instead of checking your next message, Zeron compacts as soon as a turn ends past the trigger and nothing is queued. The trigger is **Check from**, or 85% of the point where the agent's own CLI would compact by itself, whichever comes first. For Claude that point is the window set in the model picker minus 33K, so a 200K chat triggers at about 142K and a 1M chat with a 400K window at the 150K check, long before 367K.

How it compacts depends on the agent:

- **Claude Code** compacts in place right after the reply, with the last two turns carried over word for word. No model is asked first. Background shells and subagents keep running. The card under the reply reads **Compacting · Carry over · 212K · 0:30, usually ~0:53**, then **Compacted**. A message you send meanwhile waits in the queue, marked **Sends after compaction · 0:23 left**, and goes out with the carried turns as soon as the compaction ends.
- **Codex** forks the conversation at the last reply and compacts the copy on a second Codex process, while the chat itself keeps working. The session footer shows **Compacting a copy · 0:23**. When the copy is ready and the chat is idle, Zeron swaps it in: the next turn resumes from the compacted copy, and any turns that ran after the fork go along as text (their tool calls, output and thinking do not). The card reads **Compacted in the background · Swapped in · 212K → 69K · 1 turn as text**. While a turn runs the chip reads **Copy ready · swaps when the turn ends**; while background work runs, **Copy ready · waits for background work**, because the swap restarts Codex. If the chat compacts some other way first, or Codex compacts by itself mid-turn, the copy is thrown away. Codex cannot take a focus for its summary.
- Other agents keep the behavior described above.

A chat that could not compact at the turn end, because a message was already queued or the agent was busy, is still checked when you send, as described in **Compact new tasks**.

## Which messages are checked

Only a message you type yourself, before a new turn starts. These are never classified:

- slash commands such as `/compact`
- messages sent while a turn is already running
- messages from another thread
- handbacks from delegated tasks and from a Task's steps
- your answers to an agent's question, and wakes when another thread asks something
- the Supervisor's wakes
- the agent's own next-message note, which is checked as described above
- the first message after the agent compacted itself
- the first message after the chat moved to another agent: the conversation handed over replaces the compaction
- a message while background work is still running on an agent that restarts its process to compact, which would end that work

## Check from and Remind at

The same row reads **Check from** in Classifier mode and **Remind at** in Reminder mode. Zeron counts only the context a compaction could free: what the chat holds above what was left after its last compaction. Below the setting, nothing is checked and no reminder is sent.

If 70% of what the chat's window leaves is lower than the setting, checking starts at that 70% instead, so a chat with a smaller window still gets checked. For Claude, the window is the one set by the **Context** card in the model picker. **Off** turns both the classifier and the reminders off.

## Classifier source and model

In Classifier mode, the **Classifier** row picks who reads your messages.

**Agent model** runs the check on one of your agents, through its own account. By default that is the chat's own agent when it is Claude Code or Codex, otherwise the first of them that is turned on, using its newest Sonnet model when it offers one. The **Classifier model** row picks another agent and model. Sonnet-class is recommended: a small model compacted follow-ups in testing. Each check is a short extra request to that agent's provider.

**Jev · TypeSafe** and **Jev · OpenRouter** use TypeSafe's Jev 1.14 classifier instead. It is billed to your TypeSafe or OpenRouter account. In Zeron's testing a check took under a second and cost about a hundredth of a cent. Jev is offered on macOS, where the key is kept. It needs an API key from the source you picked: the row says where to get one (console.typesafe.ai/keys or openrouter.ai/settings/keys). Copy the key and press **Paste key** to save it in the Keychain, or **Remove key** to delete it. Without a key, checks give no answer and the chat is not compacted.

What leaves your Mac for each check: your last four messages, each with the agent's last written reply, every piece clipped to its last 800 characters, plus your new message. With Jev, that text is sent to TypeSafe or OpenRouter, clipped to its last 10,000 characters. Tool output and files are never sent.

## In tasks

**In tasks** decides whether the lead and step threads of a Task with a plan are compacted automatically. On (the default), they follow the same rules as any chat. Off leaves them alone: no idle compaction, no reminders, and no classifier check. A lead stays a Task's thread after its Task is complete. The agent's own compact tool still follows **Let agents compact themselves**, and you can still compact by hand. See [Agent trees and plans](zeron://docs/mission-control) for Tasks with a plan.

## What you see in the transcript

Each classifier check shows as one card in the transcript that follows the compaction through:

1. **Checking whether to compact**, or **Checking the agent's note**, with the classifier's model and the seconds so far.
2. **Compacting** with the answer, such as **Carry over** or **New task**, the context size and the time taken so far. The classifier's reason and a progress bar sit under it.
3. **Compacted** with the context before and after, such as "120.0k → 40.2k". The card then slims to one line.

With **Compact ahead of time** on, the same card follows a compaction at the end of a turn: **Compacting** with **Carry over** right under the reply, then **Compacted**. A Codex copy that was swapped in reads **Compacted in the background** with **Swapped in** and how many turns went along as text.

When the classifier answers **Same task**, the card reads **Not compacted** and the context is kept; **No answer** reads **Not compacted** too. A compaction that runs without asking, such as after three **Same task** answers in a row or with the context near the window, says why in place of the reason.

Click the card to open it. **What the agent kept** has the summary the agent wrote and what was carried over word for word. **Classifier** has the decision, the reason, the model and how long it took, the context size, and the tokens used. **What the classifier saw** shows exactly what was sent, and the reply.

Other compactions are marked by a divider, with the context before and after when the agent reports it:

| Divider | Cause |
| --- | --- |
| **Context compacted** | You compacted by hand. |
| **Compacted while idle, before the prompt cache expired** | Idle compaction. |
| **Agent compacted** | The agent's compact tool. |
| **Fresh session before a new task** or **Fresh session, last turns carried over** | A classifier answer, for an agent that cannot compact. |

When the agent wrote a summary, click the divider to read it.

## Settings

In [Settings > General](zeron://settings/general):

| Row | Default |
| --- | --- |
| **Compact idle chats before the cache expires** | On |
| **Idle threshold** (100K, 150K, 200K) | 100K |
| **Let agents compact themselves** | On |
| **Compact new tasks** (Reminder, Classifier) | Classifier |
| **Compact ahead of time** (Experimental) | Off |
| **In tasks** | On |
| **Check from** or **Remind at** (Off, 100K, 150K, 200K) | 150K |
| **Classifier** (Agent model, Jev · TypeSafe, Jev · OpenRouter), Classifier mode only | Agent model |
| **Classifier model**, with Agent model | Sonnet-class |

## Turn it all off

Switch off **Compact idle chats before the cache expires** and **Let agents compact themselves**, and set **Check from** or **Remind at** to **Off**. Zeron then compacts a chat only when you ask.

## Long context for Claude

For Claude models that support it, the model picker has a **Context** card. **Long context** switches between a 1M and a 200K window, and its slider sets when the chat compacts. Either change starts a new Claude process on your next message.
