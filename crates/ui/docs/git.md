The Git tab shows a project's commit history and working tree, and lets you stage, commit, pull, and push.

## Open the Git tab

Click **Pull requests** in the rail, then switch the pill at the bottom of the page from **PRs** to **Git**. Both tabs share the repository menu, so pick a project there first.

The checkout chip in the toolbar chooses what you look at: **Main checkout**, or one of the **Agent worktrees** your chats created. **Pull** and **Push** sync the branch with its upstream; Push asks first.

## History and the commit panel

The main area is the commit graph, with **Fetch all**, commit search, and a toggle for branch tips only.

Click a commit to open the commit panel. It shows the author, date, parents, and SHA, plus **Files** with each diff inline. From there you can **Checkout**, **Cherry-pick**, **Copy SHA**, or **Create branch** at that commit. Drag the panel's left edge to resize it, and double-click the edge to reset it.

## Commit

Click the top row, **Uncommitted changes**, click the commit button next to the **PRs** | **Git** pill, or press C when nothing has focus. The composer opens with the file tray:

- **Staged** and **Changes** list your files. Tick a file to stage it, or use **Stage all**. Click a file to see its diff.
- Click **Stash**, then **Stash all**, to set your changes aside. They show under **Stashed** with **Pop** and **Delete**.
- Type a message in the box and press Cmd+Enter to commit the staged files. Plain Enter adds a new line.
- Click **Write with AI** to draft the message from the staged diff. The model chip beside it picks a model for this commit only.
- The chevron next to the composer's commit button offers **Commit**, **Amend**, and **Commit all**.

When an agent is working in the same checkout, a banner says what the Git tab can still do.

## Settings > Source Control

[Settings > Source Control](zeron://settings/source-control) has three sections:

| Section | Rows |
| --- | --- |
| **Commit message** | **Instruction** (presets **Default**, **Conventional**, **Match repo style**) and **Default model** |
| **Pull Requests · AI chat** | **Chat history** and **New chat when idle**, for the PR chat |
| **Git** | **While an agent is running**: **Safe actions only**, **Read-only**, or **Allow everything** |

The gear next to **Write with AI** opens this page.

> Each chat's own changes are also in the right pane: add **Diffs** or **History** from its tab menu. See [Right pane](zeron://docs/right-pane).
