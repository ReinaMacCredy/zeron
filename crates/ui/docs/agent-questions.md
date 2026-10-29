Agents sometimes stop to ask you something: a question about the task, or permission to run a command or edit a file. Zeron shows these where you are already looking and marks every chat that is waiting on you.

## Answer in the current chat

When the agent in the open chat asks, a question panel takes the place of the composer. It shows the question, numbered options, and a text box. With several questions, a counter shows which one you are on.

![Answering an agent's question](agent-questions-panel.jpg)

- Click an option, or press its number (1 to 9). A single-choice question moves on by itself.
- A question marked **Select one or more options.** lets you pick several before you continue.
- To answer in your own words, type in the text box instead. Typed text replaces the picked options.
- Click **Next**, or **Submit** on the last question. Enter does the same. From the second question on, **Back** returns to the one before; with the text box empty, Escape does the same.

OpenCode's questions use this panel too. OpenCode 2 asks through a form in its session, and Zeron shows that form here as ordinary questions, single-choice or multiple-choice.

## Questions from other chats

When an agent in another chat is waiting, a pill appears above the main composer with that chat's name and **Input**, or the number of waiting questions. Click it to open the tray and answer without leaving your chat. **Open thread** jumps to the chat that asked. Closing the tray keeps your picks and text for later.

![A question from another chat](agent-questions-pill.jpg)

## When an agent needs a secret

An agent that needs an API key, a token or a password asks for it with Zeron's `request_secret` tool instead of asking you to paste it into the chat. The transcript shows **Asking for a secret** and a card with what the agent needs and where to get it.

![An agent asking for a secret](agent-secret-card.jpg)

- Type or paste the secret in the field. It shows only dots, and it cannot be copied back out.
- Click **Save securely**, or press Enter. **Decline** tells the agent you chose not to.
- The secret goes to a private file on this Mac that only your user account can read. The agent gets the file's path, never the secret, and reads it inside its commands. The file is deleted after 24 hours.
- The chat records only what happened: **Secret saved**, **Declined**, **Timed out** (no answer within the agent's wait, 10 minutes unless it set another), or **Cancelled** (the turn ended first). The secret never enters the chat, its history on your other devices, or the logs.

Enter the secret on the Mac that runs the chat. Your iPhone shows the request but asks you to enter it on your Mac, since answers typed on a phone travel through the chat itself. A secret request from another chat shows in the question pill; **Open thread** takes you to its card.

## Approvals and permission modes

A permission request is a question with fixed answers, such as **Allow** and **Deny** for Claude Code or **Yes** and **No** for Codex.

How often agents ask is set by the **Permissions** row in the model picker, under the effort slider: click a mode, or hover one to read what it does. The permission chip under the composer opens the same choices, under **Approve agent actions**. The New chat canvas always shows the chip; in a chat it shows only when the mode is not **Full access**:

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
