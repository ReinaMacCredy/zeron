Every agent has a context window, the amount of conversation it can hold at once. Compaction replaces the older conversation with a summary so the chat can keep going and later turns stay cheaper.

## See how full the context is

The ring at the right of the session footer shows how much of the context window is in use. It turns to a warning color at 75% and to danger at 90%. Click it to open **Context window**, with the tokens used and remaining.

## Compact by hand

Click **Compact context** in that popover, or type `/compact` in the composer. You can add a focus for the summary:

```
/compact keep the API design decisions
```

Claude Code, Codex, OpenCode, Pi, and ACP agents that offer it can compact. Otherwise, or while a turn is running, the button is dimmed and says why. A divider such as **Context compacted** marks the spot in the transcript.

## Idle compaction

Claude can keep a chat's prompt cache for 1 hour. Once it expires, your next message makes Claude reprocess the whole chat. With **Compact idle chats before the cache expires** on, Zeron summarizes a long idle chat just before that, so coming back reprocesses the summary instead. Chats smaller than the **Idle threshold** are left alone, and the chat must be idle with no subagents or background tasks running. The context popover shows when it **Compacts at**; sending a message first cancels it.

When you open a large Claude chat whose cache has already expired, a **Resume with less context** card offers **Compact** or **Keep full history**.

## Agent self-compaction

With **Let agents compact themselves** on, agents get a compact tool and can summarize their own context, for example before starting a new task. **Compact new tasks** chooses how:

- **Classifier** (default): a model reads each message and compacts first when it starts a new task. **Check from** sets the context size where checking starts, and **Classifier model** picks the model.
- **Reminder**: each message carries a reminder and the agent decides. **Remind at** sets when the reminders start.

## Settings

In [Settings > General](zeron://settings/general):

| Row | Default |
| --- | --- |
| **Compact idle chats before the cache expires** | On |
| **Idle threshold** (100K, 150K, 200K) | 100K |
| **Let agents compact themselves** | On |
| **Compact new tasks** | Classifier |
| **Check from** or **Remind at** (Off, 100K, 150K, 200K) | 150K |

## Long context for Claude

For Claude models that support it, the model picker has a **Context** card. **Long context** switches between a 1M and a 200K window, and its slider sets when the chat compacts. Either change starts a new Claude process on your next message.
