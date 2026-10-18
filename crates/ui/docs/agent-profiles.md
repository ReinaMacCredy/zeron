An agent profile saves an agent, model, mode, and effort, plus optional instructions and a tool set, so you can start or switch a chat to that setup in one click.

## Create a profile

Open [Settings > Agent profiles](zeron://settings/agent-profiles) and click **New profile**. The editor has these fields:

| Field | What it sets |
| --- | --- |
| **Name** | The name shown in the picker and on the chip. |
| **Agent**, **Model**, **Mode**, **Effort** | The setup to apply. **Keep current** leaves that setting as it is. |
| **Tools** | Which Zeron tools, built-in tools, and MCP servers stay on. |
| **Instructions** | Added to the agent's system prompt on every run of a chat using this profile. |
| **When to use** | A note agents read to pick a profile when they delegate a task. |

**Mode** is **Ask**, **Auto**, or **Full**, the same choices as the permission chip. Click **Save** to keep it. To delete a profile, open it with **Edit**, click **Remove**, then **Confirm remove**. The arrow buttons on each row change the order.

Profiles an agent wrote for you carry a **by agent** badge.

**System prompt**, below the list, is text added to every agent's system prompt on this device, before a profile's own instructions.

## Apply a profile

Open the model picker (Cmd+/). Profiles have their own tab in the full picker; in the compact picker, click **Switch to full picker** first. Search with **Search profiles…** and click a profile to apply it. **No profile** keeps the model and drops the profile's instructions. **Save current as profile** turns your current pick into a new profile.

If the chat already has history and the profile uses another agent or model, Zeron asks **Switch provider?** or **Switch model?** first. See [Models and effort](zeron://docs/models-and-effort).

## The profile chip

With a profile applied, the model chip in the composer splits in two. The left half shows the profile name and opens the profile list. The right half shows the model and effort and opens the normal model picker.

## When a chat drifts

If you change the model, effort, mode, tools, or instructions after applying a profile, the chip shows a small warning dot and the profile row reads **Edited** with what changed. Click **Revert** to apply the profile again, or **Save** to write your current settings into the profile. An unchanged profile reads **Active**.

> Changing a chat's instructions, by switching profile or editing the system prompt, restarts a running Claude, Codex, or ACP session on your next message so the new instructions take effect.

## Profiles and new threads

Threads that agents open for you do not inherit the creating chat's instructions unless the agent names a profile.
