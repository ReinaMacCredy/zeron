The **Live state** column sits on the right of a chat's conversation. It sums up what the chat has open and running: its workspace, side chats, actions, subagents, attachments, and usage. Click a row to jump to what it names.

![The Live state column](live-state-column.jpg)

## Show and hide it

The chevron at the top collapses the column to a narrow rail of icons, and the rail's chevron (**Expand live state**) brings it back. Zeron remembers your choice. Hover a rail icon to see its label and value; click it to do what the row does.

The column collapses to the rail by itself when the conversation is narrower than about 640 pixels, and whenever the right pane is open. With the right pane open, the rail's chevron opens the same content as a **Live state** popover instead (**Close live state** closes it).

The column is not shown on New chat or while a [split view](zeron://docs/split-view) is open. Each section shows at most 8 rows, except **Actions** and a section's **Previous agents** fold, which list all of theirs.

## On your project

The first section is titled **On** and the project's name, or **On this session** for a chat without a project.

| Row | What it does |
| --- | --- |
| **Changes** | Git projects only. Opens the **Working tree** diff, or the first Diffs tab you have open. While a Working tree tab is open, the row shows the number of changed files and lines. See [Right pane](zeron://docs/right-pane). |
| **Browser** | Shows the first browser tab, or opens one. See [Browser](zeron://docs/browser). |
| **Terminal** | Shows the first terminal tab in the right pane, or opens one. Shows **N running** while pane terminals are running. See [Terminal](zeron://docs/terminal). |
| **Files** | Opens the Files panel. |
| **Computer** | While an agent uses [computer use](zeron://docs/computer-use) in this chat: the app it is in, with a status dot. Click to show or hide the preview. |
| A device | One row per device open for this chat, marked **You** or **Agent** for who opened it. Click to show it. See [Device hub](zeron://docs/device-hub). |

## Workspace

The device the chat runs on (or **Local**) with its folder (or **No workspace**). In a git project, a second row shows the branch (or **No branch**) and the short commit it is on. These rows are for reading only.

## Local servers

One row per browser tab, with its page title and host. Click a row to show that tab.

## Side chats

Your `/btw` sidechats of this chat, each with its status or **Expired**. Click one to open it in the right pane.

- **New sidechat** starts another sidechat in the right pane.
- **Fork chat** copies this conversation into a new chat in the right pane that carries on by itself.

See [Side chat](zeron://docs/side-chat).

## Actions

The project's saved Actions, each with **Running**, **Done**, or **Failed** once you have run it. Click a row to run it, hover it for **Edit action**, or click **Add action**. See [Projects](zeron://docs/projects).

## Subagents

The heading reads **Subagents**, or **Subagents · N running**. Finished agents fold under **Previous agents (N)**. Click a row to open that agent. See [Delegation](zeron://docs/delegation).

## Sources

Every file and image you attached to a message in this chat. Click one to preview it.

## Usage

How much of the chat agent's plan is used, on its busiest limit window, as a meter. It reads **Usage unavailable** when Zeron has no usage figures for the account. See [Usage](zeron://docs/usage).

## Project

The repository's name, or **No repository**. When the checkout has an `AGENTS.md` or `CLAUDE.md` at its root, **Project instructions** opens it in the right pane. Below that, one row per file tab open in the right pane; click to show it.
