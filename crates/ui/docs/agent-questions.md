Agents sometimes stop to ask you something: a question about the task, or permission to run a command or edit a file. Zeron shows these where you are already looking and marks every chat that is waiting on you.

## Answer in the current chat

When the agent in the open chat asks, a question panel takes the place of the composer. It shows the question, numbered options, and a text box. With several questions, a counter shows which one you are on.

- Click an option, or press its number (1 to 9). A single-choice question moves on by itself.
- A question marked **Select one or more options.** lets you pick several before you continue.
- To answer in your own words, type in the text box instead. Typed text replaces the picked options.
- Click **Next**, or **Submit** on the last question. Enter does the same. With the text box empty, Escape goes back a question.

## Questions from other chats

When an agent in another chat is waiting, a pill appears above the main composer with that chat's name and **Input**, or the number of waiting questions. Click it to open the tray and answer without leaving your chat. **Open thread** jumps to the chat that asked. Closing the tray keeps your picks and text for later.

## Approvals and permission modes

A permission request is a question with fixed answers, such as **Allow** and **Deny** for Claude Code or **Yes** and **No** for Codex.

How often agents ask is set by the permission chip in the composer footer, under **Approve agent actions**:

| Mode | What it does |
| --- | --- |
| **Ask for approval** | Always ask before tools and commands |
| **Approve for me** | Only ask for actions flagged as unsafe |
| **Full access** | Any file, any command, no prompts |

**Full access** is the default. The mode applies to the selected chat, or to the next new chat.

## Where waiting chats show up

- In the sidebar, a waiting chat shows **Input**. A message that reaches the chat while it waits, such as another agent's message, does not clear it; **Input** stays until you answer.
- The rail's Inbox button shows a count of waiting chats, and the [Inbox](zeron://inbox) lists them under **Needs you**.
- On the [Tasks](zeron://tasks) board, a started task whose chat stops working, to ask or to finish, moves to **Needs you**.
- A desktop notification reads **Waiting on your input**. See [Notifications](zeron://docs/notifications) to set it up.

> Computer use asks for its own consent inside the transcript. See [Computer use](zeron://docs/computer-use).
