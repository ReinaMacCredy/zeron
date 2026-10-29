Every conversation starts on the new chat canvas: a composer in the middle of the window where you choose where the agent works, then type your first message.

## Open a new chat

- Press Cmd+N (**New session** in [Settings > Shortcuts](zeron://settings/shortcuts)).
- Click the square-pen button at the top of the sidebar (tooltip **New session**).
- With the sidebar organized **By project**, hover a project's group and click its **+** (**New chat in project**) to start on that project. The **Chats** group and the other layouts have no **+**.
- Open the command palette with Cmd+K and pick **New chat**.
- Type `/new` in any composer.

If the canvas you are on is still empty, Zeron reuses it. If you already typed something, that canvas is kept as a draft and a fresh one opens.

## Chat or Work

The **Chat | Work** toggle sits at the top of the canvas.

- **Work** runs the agent in a project. It returns to the project you used last. If you have no project yet, Zeron opens the add-project flow.
- **Chat** runs the agent outside any project, in a scratch folder under `~/.zeron/scratch`. The placeholder changes to "What's on your mind?".

Zeron remembers your choice for the next canvas.

## Pick the project, device, and checkout

Chips above the composer show the target. Click the project chip to search your projects (**Search projects…**). At the bottom of that list, **New project** adds one (Cmd+Shift+N) and **Don't work in a project** switches to a projectless chat. The device chip picks which machine runs the agent (**This device** by default).

![The new chat canvas with project, device, and checkout chips](new-chat-canvas.jpg)

For a git project, chips below the composer pick the checkout and branch. Choose **Current checkout** to work in place, or **New worktree** to have Zeron cut a fresh worktree and branch for this chat.

To choose the agent and model, see [Models and effort](zeron://docs/models-and-effort).

## Send the first message

Press Enter (or Cmd+Enter, depending on **Send messages with** in [Settings > Chat](zeron://settings/chat)). The canvas becomes the new thread. Until it is named, the thread shows as **New session** in the sidebar.

**Thread naming** in Settings > Chat decides who writes the title. It reads **Session agent** by default: a Claude Code or Codex chat is named by its own agent on that agent's cheapest model, and any other chat by the first enabled of Claude Code or Codex. Click the chip to pick one model for every title instead, and **Reset** to go back to **Session agent**. If no title comes back, the thread is named after the first words of your prompt. A thread is named once; rename it yourself from the sidebar.

> With **Send messages with** set to Enter, Cmd+Enter starts the thread in the background and leaves you on a fresh canvas with the same target.

## Start a mission

Above the composer, **New task**, **Mission Control** and **Running agents** are tabs. Click **Mission Control**, or press Shift+Tab in the composer, to start a [Mission Control task](zeron://docs/mission-control) instead of a chat. On a list line, Shift+Tab still moves the line out a level. The composer's border takes the accent color, the placeholder asks you to describe the mission, and two choices replace the paperclip:

- **plan.md** or **Steps only** picks what the lead writes. **plan.md** is the default.
- **Review plan first**, on by default, makes the lead wait for your approval before it runs any step.

![Starting a mission from the new chat canvas](new-chat-mission.jpg)

Type the mission and click **Start mission**, or press the send key. The first line becomes the task's title and the rest its notes. Zeron creates the task in the project you picked, starts its lead with the agent, model, effort and permissions shown in the composer, and opens the task's page. Press Shift+Tab again (**⇧Tab: back to New task** under the composer) or click **New task** to go back to a chat.

The **Running agents** tab, with the number of working chats beside it, swaps the composer for a list of those chats: each row shows the title and how long it has been running. Click a row to open that chat. With nothing running it says **No agents running**.

You can also turn an open chat into a mission from its own composer. See [Mission Control](zeron://docs/mission-control).

## Drafts

A canvas with text, attachments, or Appshots is a draft. Leave it and it stays in the **Drafts** section of the sidebar, right under Pinned, and it survives a restart. Each row shows the target (project and device) and the start of your prompt. Click a row to pick up where you left off, or hover it and click the **×** to discard it. An empty canvas is dropped when you leave.
