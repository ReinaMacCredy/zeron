Every conversation starts on New chat: a composer in the middle of the window where you choose where the agent works and type your first message, over three cards with what needs you, what is running and what you did last.

## Open a new chat

- Click **New chat**, the top button of the app rail (tooltip **New chat · ⌘N**).
- Press Cmd+N from anywhere, a chat included (**New session** in [Settings > General > Shortcuts](zeron://settings/shortcuts)).
- With the sidebar organized **By project**, hover a project's group and click its **+** (**New chat in project**) to start on that project. The **Chats** group and the other layouts have no **+**.
- Open the command palette with Cmd+K and pick **New chat**.
- Type `/new` in any composer.

If the New chat you are on is still empty, Zeron reuses it. If you already typed something, it is kept as a draft and a fresh one opens.

## Pick the project, device, and checkout

The project chip in the composer bar (tooltip **Where the agent works**) names the project, or **No project**. Click it to search your projects (**Search projects…**). At the bottom of that list, **New project** adds one (Cmd+Shift+N) and **Don't work in a project** runs the agent outside any project, in a scratch folder under `~/.zeron/scratch`; the placeholder then reads "What's on your mind?". Under the composer, the device chip picks which machine runs the agent (**This device** by default). The access chip on the right of the composer bar names the access mode (**Supervised**, **Auto-accept edits** or **Full access**); see [When an agent asks](zeron://docs/agent-questions).

![New chat: the composer with its project chip and Plan switch, the device, branch and checkout chips, and the Needs you, Running and Recent cards](new-chat-canvas.jpg)

For a git project, chips below the composer pick the checkout and branch. Choose **Current checkout** to work in place, or **New worktree** to have Zeron cut a fresh worktree and branch for this chat.

To choose the agent and model, see [Models and effort](zeron://docs/models-and-effort).

## Needs you, Running and Recent

Under the composer, three cards sum up your other threads:

- **Needs you** lists the threads that ask a question, have a plan waiting for your OK, or failed, like the Cmd+I tray. **Answer** and **Review** open the question or the plan right in the card; **Open** opens the thread. Past three rows, **N more in the tray** opens the tray.
- **Running** lists the threads that are working, with their agents.
- **Recent** lists your finished chats, newest first.

Click a row to open its thread. The other Home widgets (pull requests, Git activity, this Mac, running services, CI failures) fold under the pill below the cards; see [Home widgets](zeron://docs/home-widgets).

## Send the first message

Press Enter (or Cmd+Enter, depending on **Send messages with** in [Settings > General > Chat](zeron://settings/chat)). New chat becomes the new thread. Until it is named, the thread shows as **New session** in the sidebar.

**Thread naming** in Settings > General > Chat decides who writes the title. It reads **Session agent** by default: a Claude Code or Codex chat is named by its own agent on that agent's cheapest model, and any other chat by the first enabled of Claude Code or Codex. Click the chip to pick one model for every title instead, and **Reset** to go back to **Session agent**. If no title comes back, the thread is named after the first words of your prompt. A thread is named once; rename it yourself from the sidebar.

> With **Send messages with** set to Enter, Cmd+Enter starts the thread in the background and leaves you on a fresh New chat with the same target.

## Start with a plan

Turn on the **Plan** switch in the composer bar, or press Shift+Tab, to have the agent write a plan before it changes anything. On a list line, Shift+Tab still moves the line out a level. The composer's border takes the accent color, the placeholder reads "Describe the goal. The agent writes a plan and waits for your OK.", and the send button reads **Start plan**.

![New chat with Plan on: the accent border and Start plan](new-chat-plan.jpg)

The chat starts in plan mode. When Claude Code's plan is ready it arrives as a card with **Approve**, **Approve as Task** and **Edit plan**; see [The composer](zeron://docs/composer). On Codex and OpenCode the agent writes a plan and changes nothing, without an approval step. The next New chat starts with Plan off.

## Drafts

A New chat with text, attachments, or Appshots is a draft. Leave it and it stays in the **Drafts** section of the sidebar, right under Pinned, and it survives a restart. Each row shows the target (project and device) and the start of your prompt. Click a row to pick up where you left off, or hover it and click the **×** to discard it. An empty New chat is dropped when you leave.
