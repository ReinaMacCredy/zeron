Every conversation starts on the new chat canvas: a composer in the middle of the window where you choose where the agent works, then type your first message.

## Open a new chat

- Press Cmd+N (**New session** in [Settings > Shortcuts](zeron://settings/shortcuts)).
- Click the square-pen button at the top of the sidebar (tooltip **New session**).
- Hover a project group in the sidebar and click its **+** (**New chat in project**) to start on that project.
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

For a git project, chips below the composer pick the checkout and branch. Choose **Current checkout** to work in place, or **New worktree** to have Zeron cut a fresh worktree and branch for this chat.

To choose the agent and model, see [Models and effort](zeron://docs/models-and-effort).

## Send the first message

Press Enter (or Cmd+Enter, depending on **Send messages with** in [Settings > General](zeron://settings/general)). The canvas becomes the new thread. Until it is named, the thread shows as **New session** in the sidebar. **Thread naming** in Settings > General decides who writes the title.

> With **Send messages with** set to Enter, Cmd+Enter starts the thread in the background and leaves you on a fresh canvas with the same target.

## Drafts

A canvas with text, attachments, or Appshots is a draft. Leave it and it stays in the **Drafts** section of the sidebar, right under Pinned, and it survives a restart. Each row shows the target (project and device) and the start of your prompt. Click a row to pick up where you left off, or hover it and click the **×** to discard it. An empty canvas is dropped when you leave.
