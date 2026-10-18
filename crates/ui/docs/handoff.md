Handoff moves a conversation to another agent. You can switch agents in the same chat, continue in a fresh thread with any agent, or hand a loose chat to a project.

## Open the handoff menu

Click **Hand off thread**, the arrows button in the titlebar of a chat. It is available once the chat has a finished message and the agent is not working or waiting on you.

The menu has up to three sections:

| Section | What it does |
| --- | --- |
| **Continue in this thread** | Switches this chat to another agent. Lists every agent except the current one. |
| **Continue in a new thread** | Opens a new chat with the agent you pick, the current one included. |
| **Hand off to project** | Shown for a chat without a project. Starts a thread in the project you pick. |

Each row names the agent and the model it will use: the model you picked last for that agent, or its default. Change the model afterwards in the model picker.

## Continue in this thread

Zeron asks **Switch provider?** first. Confirm, and the conversation so far goes to the new agent as plain text with your next message. A divider marks where the previous agent stopped. See [Models and effort](zeron://docs/models-and-effort) for more on switching.

## Continue in a new thread

The new chat opens in the same folder and branch, titled after the source with **(cont.)** added. It starts from a handoff packet: the earlier conversation as plain text, without tool results or file state. Long conversations are trimmed to fit.

## Hand off to project

The new thread runs in the project's folder. Instead of a text copy, it gets a link to the source chat, and the agent reads the source with Zeron's thread tools. The source chat notes where it was handed off to.

## The handoff divider

The new thread opens with a divider reading **Continued from** and the source title, with chips naming the agent it came from and the agent it went to. Click the divider to expand it and see **From**, **To**, **Context**, and the **Transferred context** itself.

> Handoff keeps the conversation, not the runtime: open files, tool output, and the old agent's prompt cache stay behind.
