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

When an agent in another chat is waiting, the pill at the right of the titlebar counts it, for example **1 need you · 2 running**. Click the pill or press Cmd+I to open the tray. Under **Needs you**, each waiting thread has a row with what it waits on: **Asking**, **Asking 1/2** while it has several questions, **Plan ready**, or **Failed**. A delegated child's row names its parent (**from** and the parent's name). **Answer** opens the same question panel inside the tray, and **Review** opens a plan with **Approve as Task** and **Approve**; you stay in your chat, and the row folds away once it is answered. Click a row's title to open the chat that asked. Closing the tray keeps your picks and text for later. Esc, a click outside, or Cmd+I closes it.

The **Needs you** card under the composer on New chat lists the same rows, with the same **Answer** and **Review**.

![A question from another chat, answered in the Cmd+I tray](triage-tray.jpg)

## When an agent needs a secret

An agent that needs an API key, a token or a password asks for it with Zeron's `request_secret` tool instead of asking you to paste it into the chat. The transcript shows **Asking for a secret** and a card with what the agent needs and where to get it.

![An agent asking for a secret](agent-secret-card.jpg)

- Type or paste the secret in the field. It shows only dots, and it cannot be copied back out.
- Click **Save securely**, or press Enter. **Decline** tells the agent you chose not to.
- The secret goes to a private file on this Mac that only your user account can read. The agent gets the file's path, never the secret, and reads it inside its commands. The file is deleted after 24 hours.
- The chat records only what happened: **Secret saved**, **Declined**, **Timed out** (no answer within the agent's wait, 10 minutes unless it set another), or **Cancelled** (the turn ended first). The secret never enters the chat, its history on your other devices, or the logs.

Enter the secret on the Mac that runs the chat. Your iPhone shows the request but asks you to enter it on your Mac, since answers typed on a phone travel through the chat itself. A secret request from another chat shows in the tray; click its row to open the chat with its card.

## Approvals and access modes

A permission request is a question with fixed answers, such as **Allow** and **Deny** for Claude Code or **Yes** and **No** for Codex.

How often agents ask is set by the access chip in the composer bar, before the model chip. It names the current mode; click it to pick one of the modes under **Permissions**. The chip shows in every composer except a pull request chat, on New chat and in a chat alike. In a narrow chat it shows only its icon, with the mode in its tooltip:

| Mode | What it does |
| --- | --- |
| **Supervised** | Ask before commands and file changes. |
| **Auto-accept edits** | Apply edits, ask before other actions. |
| **Full access** | Run commands and edits without asking. |

**Full access** is the default. The mode applies to the selected chat, or to the next new chat. Only the modes the chat's agent supports are listed: Grok, Devin, Hermes and other ACP agents can't ask before acting, so they always run with **Full access**. fx is the exception: it offers **Supervised** and **Full access**.

**Auto-accept edits** applies file changes on its own and still asks before commands. Chats set to **Ask for approval**, **Approve for me** or **Full** in earlier versions open as **Supervised**, **Auto-accept edits** and **Full access**.

The **Plan** switch next to the access chip, or Shift+Tab, has the agent explore and propose a plan before changing anything. In a Claude Code chat, Send then reads **Start plan**, and the plan arrives as a card with **Approve**, which runs it in this chat in the chat's mode, **Approve as Task**, which tracks its steps in [Tasks](zeron://docs/tasks), and **Edit plan**, which tells the agent no so you can say what to change. Codex and OpenCode write a plan and change nothing; they show no card.

## Where waiting chats show up

- In the sidebar, a waiting chat shows **Asking** (**Asking 1/2** with several questions), or **Plan ready** for a plan that waits for your approval. A message that reaches the chat while it waits, such as another agent's message, does not clear it; **Asking** stays until you answer.
- The pill at the right of the titlebar counts waiting chats, and the Cmd+I tray lists them under **Needs you**. While any wait, the [Inbox](zeron://inbox) shows a banner with **Open tray**.
- On the [Tasks](zeron://tasks) board, a started task whose chat stops working, to ask or to finish, moves to **Needs you**.
- A desktop notification reads **Waiting on your input**; clicking it opens the tray with that chat's row open. See [Notifications](zeron://docs/notifications) to set it up.

> Computer use asks for its own consent inside the transcript. See [Computer use](zeron://docs/computer-use).
