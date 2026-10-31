The Git tab shows a project's commit history and working tree, and lets you stage, commit, pull, and push.

## Open the Git tab

Click **Pull requests** in the rail, then switch the pill at the bottom of the page from **PRs** to **Git**. Both tabs share the repository menu, so pick a project there first.

The checkout chip in the toolbar chooses what you look at: **Main checkout**, or one of the **Agent worktrees** your chats created. **Pull** and **Push** sync the branch with its upstream; Push asks first.

## History and the commit panel

The main area is the commit graph, with **Fetch all**, commit search, and a toggle for branch tips only.

![The Git tab with a commit open](git-commit-panel.jpg)

Click a commit to open the commit panel. It shows the author, date, parents, and SHA, plus **Files** with each diff inline. From there you can **Checkout**, **Cherry-pick**, **Copy SHA**, or **Create branch** at that commit. Drag the panel's left edge to resize it, and double-click the edge to reset it.

## Commit

Click the top row, **Uncommitted changes**, click the commit button next to the **PRs** | **Git** pill, or press C when nothing has focus. The commit button's badge counts your changed files, and a dot on it means you left an unsent commit message. The composer opens with the file tray:

- **Staged** and **Changes** list your files. Tick a file to stage it, or use **Stage all**. Click a file to see its diff.
- Click **Stash**, then **Stash all**, to set your changes aside. They show under **Stashed** with **Pop** and **Delete**.
- Type a message in the box and press Cmd+Enter to commit the staged files. Plain Enter adds a new line.
- Click **Write with AI** to draft the message from the staged diff. The model chip beside it picks a model for this commit only.
- The chevron next to the composer's commit button offers **Commit**, **Amend**, and **Commit all**.

![Writing a commit](git-commit.jpg)

When an agent is working in the same checkout, a banner says what the Git tab can still do.

## Settings > Agents > Source control

[Settings > Agents > Source control](zeron://settings/source-control) has three sections:

| Section | Rows |
| --- | --- |
| **Commit message** | **Instruction** (presets **Default**, **Conventional**, **Match repo style**) and **Default model** |
| **Pull Requests · AI chat** | **Chat history** and **New chat when idle**, for the PR chat |
| **Git** | **While an agent is running**: **Safe actions only**, **Read-only**, or **Allow everything** |

The gear next to **Write with AI** opens this page.

### Commit message

**Instruction** is sent to the model with the staged diff when you click **Write with AI**. Leave it empty to use the default. The presets fill it in:

| Preset | Instruction |
| --- | --- |
| **Default** | Write a short commit message for this staged diff. |
| **Conventional** | A Conventional Commits message (type(scope): subject) with a short bullet body. |
| **Match repo style** | The same style as the recent subjects: "Area: what changed", then a short paragraph on why. |

**Default model** reads **Automatic** unless you pick one: Zeron uses Claude Code with Haiku 4.5 when available, otherwise the cheapest model. **Reset** goes back to Automatic. The model chip next to **Write with AI** can still pick another model for one commit.

### Pull Requests · AI chat

**Chat history** decides what the [PR chat](zeron://docs/pull-requests) keeps:

- **Keep per PR** (the default): reopening a PR shows its earlier chats.
- **Fresh every time**: a new chat discards the old one unless you turn it into a thread.

Under **Keep per PR**, **New chat when idle** starts a new chat after the chat has been idle this long: **Never**, **5 min**, **10 min**, **30 min** (the default), **1 hour**, or **1 day**. Earlier chats stay in the sessions list.

### While an agent is running

This decides what the Git tab may do in a checkout where an agent is working:

| Choice | What it allows |
| --- | --- |
| **Safe actions only** (the default) | Stage, unstage, commit staged changes, create a branch without checking it out, and push, with a warning. Pull, amend, **Commit all**, and everything else wait for the agent. |
| **Read-only** | The Git tab makes no changes until the agent stops. |
| **Allow everything** | Every action runs, with a warning banner while the agent works. |

> Each chat's own changes are also in the right pane: open its **Changes** tab from the pane button at the right of the titlebar, and use the **Uncommitted** | **History** switch at its top. See [Right pane](zeron://docs/right-pane).
