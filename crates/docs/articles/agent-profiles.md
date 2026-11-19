An agent profile saves an agent, model, mode, and effort, plus optional instructions and a tool set, so you can start or switch a chat to that setup in one click.

## Create a profile

Open [Settings > Agents > Profiles](zeron://settings/agent-profiles) and click **New profile**. The editor has these fields:

| Field | What it sets |
| --- | --- |
| **Name** | The name shown in the picker and on the chip. |
| **Agent** | The agent the profile runs. A profile always sets its agent; changing **Agent** clears **Model** and the model's options. |
| **Model**, **Mode**, **Effort** | The rest of the setup. **Keep current** leaves that setting as it is. |
| Model options | Some models offer extra choices, each with its own row under **Effort**. **Keep current** leaves it as it is. |
| **Tools** | Which tools stay on. The row reads **All tools on** or how many are off; click it to open the list. |
| **Instructions** | Added to the agent's system prompt on every run of a chat using this profile. |
| **When to use** | A note agents read to pick a profile when they delegate a task. |

![Editing an agent profile](agent-profiles-editor.jpg)

**Mode** is **Supervised**, **Auto-accept edits**, or **Full access**, the same choices as the access chip in the composer, limited to the modes the profile's agent supports.

The **Tools** list has these groups:

- **Zeron tools**: the groups from [Settings > Context & tools > Tools](zeron://settings/tools), each with a switch and its single tools. A tool or group that is off in Settings > Context & tools > Tools shows **off in Settings** and cannot be turned on here.
- **Claude built-in** and **MCP servers**: only when **Agent** is Claude Code. Zeron asks Claude Code for its built-in tools and reads the MCP servers from your Claude config.
- **Not found**: tools the profile turns off that no list has any more, marked **not found · turn on to remove**. Switch one on to drop it from the profile.

Click **Save** to keep it. To delete a profile, open it with **Edit**, click **Remove**, then **Confirm remove**. The arrow buttons on each row change the order.

Profiles an agent wrote for you carry a **by agent** badge.

With no profiles yet, the page reads **No profiles yet. Add one to start chats from a saved agent and model.**

**System prompt**, below the list, is text added to every agent's system prompt on this device, before a profile's own instructions. Click its **Save** button after editing; it stays dimmed until the text changes.

## Apply a profile

Open the model picker (Cmd+/). Profiles have their own tab in the full picker; in the compact picker, click **Switch to full picker** first. Search with **Search profiles…** and click a profile to apply it. **No profile** keeps the model and drops the profile's instructions. **Save current as profile** turns your current pick into a new profile.

If the chat already has history and the profile uses another agent or model, Zeron asks **Switch provider?** or **Switch model?** first. See [Models and effort](zeron://docs/models-and-effort).

## The profile chip

With a profile applied, the model chip in the composer splits in two. The left half shows the profile name and opens the profile list. The right half shows the model and effort and opens the normal model picker.

## When a chat drifts

If you change the model, effort, mode, tools, or instructions after applying a profile, the chip shows a small warning dot and the profile row reads **Edited** with what changed. Click **Revert** to apply the profile again, or **Save** to write your current settings into the profile. An unchanged profile reads **Active**.

![The profile chip with an edited profile](agent-profiles-chip.jpg)

> Changing a chat's instructions, by switching profile or editing the system prompt, restarts a running Claude, Codex, or ACP session on your next message so the new instructions take effect.

## Profiles and new threads

Threads that agents open for you do not inherit the creating chat's instructions unless the agent names a profile.
