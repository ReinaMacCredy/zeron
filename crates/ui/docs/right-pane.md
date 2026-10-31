The right pane sits next to the chat. Its tool tabs show the chat's **Changes**, its **Files**, a **Browser** and a **Device**, and the agents, forks and side chats the chat starts open there as tabs of their own.

![The right pane's tool tabs and three child tabs, each with its kind badge](right-pane-tabs.jpg)

## Open and close

Click the pane button at the right of the titlebar, which shows the chat's changed lines (such as **+12 −3**), or press Cmd+R (**Toggle right sidebar**). Cmd+E (**Toggle files panel**) opens the pane on **Files**, and closes it again when Files is showing. Cmd+R also works on the Pull requests page. Rebind them in [Settings > General > Shortcuts](zeron://settings/shortcuts).

## Tool tabs

The tool tabs sit at the head of the pane's tab strip, before any other tab:

| Tab | Shows |
| --- | --- |
| **Changes** | The chat's diff, with **History** beside it and the **Review** button. Git projects only. See **Changes and History** below. |
| **Files** | The chat's file tree. See **Files** below. |
| **Browser** | A web browser tab. See [Browser](zeron://docs/browser). |
| **Device** | A device screen. See [Device hub](zeron://docs/device-hub). It hides once a device tab is open, since device tabs carry their own names. |

An empty pane opens on the tool you picked last, else on **Changes** in a git project or **Files**. New chat has only **Browser** and **Device**, because Changes and Files need a chat. Terminals are not pane tabs: they open in the dock under the chat (Cmd+J). See [Terminal](zeron://docs/terminal).

## Agents, forks and side chats

Every thread a chat starts opens here as a tab with a badge for its kind: an agent, a delegated task, a fork, a side chat, or a thread an agent started. It opens the same tab from every place you click it: the transcript chip, the Sidecar's **Beside this thread** list, the Files tab, the Cmd+I tray, or the command palette. A tab that is already open is brought forward instead of opened twice. A running child keeps its kind icon with a small dot on the badge.

When the tabs do not fit, the tabs you are not on shrink to their badges, and past that a **⋯ N** button lists the rest. The tab you are on always keeps its name and close button.

## Tabs

Tabs belong to each chat: switching chats shows that chat's own tabs. They last until Zeron quits. A fresh launch starts with the pane closed and no tabs. Browser logins are kept separately; see [Browser](zeron://docs/browser).

- Hover a tab for its full file path, page address, or device details.
- A spinner replaces a browser tab's icon while its page is loading.
- A dot on a file tab means unsaved edits. Hover it to get the close button.
- Drag tabs to reorder them, and middle-click to close one. Drag a file tab onto the composer to attach that file.
- After you close a tab, the tab you used most recently before it becomes active. Closing the last tab closes the pane.

Right-click a tab for **Close tab**, **Close other tabs**, **Close tabs to the left**, and **Close tabs to the right**. On a side chat or delegated child tab, the menu also has **Rename** and **Delete…** for that chat, plus **Add plan** on a delegated child that can take one. On a device tab it adds **Rename**.

Cmd+W closes the active pane tab first. When the pane has focus, Ctrl+Tab and Ctrl+Shift+Tab move between its tabs.

## Changes and History

The **Changes** tab shows the chat's own checkout. A switch at its top picks **Uncommitted** (or the scope you picked, such as **Branch changes**) or **History**, and the **Review** button on the right of that row starts a [code review](zeron://docs/review); its findings list under the switch. The Git tab on the Pull requests page is a separate view with staging and commits; see [Git](zeron://docs/git).

### Uncommitted

The menu at the left of the header picks the scope:

| Scope | Shows |
| --- | --- |
| **Working tree** (default) | Uncommitted changes, updated live |
| **Branch changes** | Everything the branch adds over a base branch, uncommitted changes included. Click the base branch name to pick another; type in **Search branches…** to filter. |
| **Latest turn** | Changes since this chat's last turn started |

The line under the header counts the changed files and added and removed lines. **Partial snapshot** means the diff was too large and was cut short.

- **Split view** shows old and new side by side; **Wrap long lines** wraps instead of scrolling sideways. Both are remembered and apply to the Changes tab and the pull request Code view.
- **Collapse all files** folds every file (**Expand all files** opens them again). Click a file's header to fold just that file.
- The document button on a file's header (**Open in file browser**) opens the file in an editor tab.
- **Discard changes** (Working tree only) asks **Discard working tree changes?** and then puts tracked files back to the last commit and deletes untracked files. Ignored files are kept. This cannot be undone. The button is dimmed when there is nothing to discard or the diff is a partial snapshot.

There is no staging here; stage and commit from the [Git](zeron://docs/git) tab.

Click a file name on a tool row in the transcript: if that file has uncommitted changes, **Changes** opens scrolled to it; otherwise the file opens in an editor tab.

### Comment on a line

Hover a line and click the **+** in the line numbers. Type in **Request a change…** and click **Comment** or press Enter (Shift+Enter adds a line, Escape cancels). In split view only the new (right) side takes comments.

![The Changes tab with review findings above the diff](right-pane-diffs.jpg)

The comment stays on its line, with an edit button and **Remove comment** on hover. Comments belong to the chat, so they show wherever that line appears.

Above the composer, a **1 comment** (or **N comments**) pill collects them. Click it to see the list, remove one with its ×, or **Clear all**. Your next message carries them: they are added under your text as a list of `file:line` notes, where L means a line in the original file and R a line in the changed one. With no text of your own, the message reads "Address the review comments below." In the transcript the message shows an **N comments** pill; hover it to read them.

Findings from a [code review](zeron://docs/review) appear on their lines in the same way, with **Fix** and **Dismiss**.

### History

**History** in the Changes tab shows the commit graph of the chat's checkout. The header shows the branch, the number of commits, and how far it is ahead of or behind its base.

- The magnifier (**Search commits**) finds a commit by SHA or subject.
- **Fetch all** fetches from every remote. The fold button switches between **Show branch tips** and **Show all commits**. Click **Refresh** to pick up new commits.
- Click a commit to see its diff. You can comment on its lines too.
- Click a SHA to copy the full SHA; it reads **Copied**.
- Hover the column headers for **Columns** to show or hide **Author**, **Date**, and **SHA**. Drag a header to reorder, or a divider to resize. Right-click **Author** and choose **Name** to show names instead of avatars.
- **Load more** loads the next 100 commits.

## Files

The **Files** tab shows the chat's file tree. Type in **Search files** to find a file or folder by name, then use Up and Down and press Enter to open it. Escape clears the search. Click a file to open it in a tab. Shift-click selects a range and Cmd-click adds or removes one item.

The eye button (**Show all files (even hidden)**) also lists files that git ignores. Files whose names start with a dot always show, and `.git` never does. It is the same switch as **Show hidden and ignored files** in [Settings > General > Files](zeron://settings/files).

Right-click for **Add to chat**, **Copy path**, **Rename…**, and **Delete…**. Delete is permanent and asks first. Drag rows onto a folder to move them, or into the chat to attach them.

### Subagents and Chats

Under the tree, two sections list what this chat has started:

- **Subagents**: the agent's built-in subagents, running ones first. Finished ones fold under **Finished**. Click a row to read its transcript in the right pane.
- **Chats**: side chats, forks, and delegated children of this chat, newest activity first. Click a row to open it in the right pane, double-click to rename it, or right-click for its menu.

Hover the **Chats** header for **New side chat** (an empty chat that starts fresh beside this one) and **Fork this chat** (a copy of this conversation that carries on by itself). Both open as tabs in this pane. See [Side chat](zeron://docs/side-chat). Each section shows 10 rows, then **Show N more**.

## Edit files

Press Cmd+S to save. Markdown files open as a preview; click **Show Markdown code** to edit the source. The header also has **Reveal file in tree**, **Open in default editor** (for files on this computer), and a word wrap toggle.

You can edit UTF-8 text files up to 1 MB inside the chat's folder. Some files open read-only or not at all, with a line saying why:

- Text files from 1 to 8 MB, files with mixed line endings, and symlinks are read-only.
- A file opened from a link outside the chat's folder shows **Read-only: outside this chat's folder.**
- Files over 8 MB, binary files, and text that is not UTF-8 do not open.
- Images open in a viewer. Pinch or Ctrl+scroll to zoom.

Closing a tab with unsaved edits, or quitting, saves it first. If the file changed on disk while you were editing, a bar reads **This file changed outside Zeron.** with **Keep Editing** and **Reload from Disk**.

[Settings > General > Files](zeron://settings/files) has:

| Setting | Default |
| --- | --- |
| **Autosave**: save a short time after your last edit | Off |
| **Autosave delay**: **300 ms**, **600 ms**, **900 ms**, **1.5 s**, or **3 s** (shown while Autosave is on) | 900 ms |
| **Word wrap**: the same switch as the header toggle | Off |
| **Show hidden and ignored files**: the same switch as the eye button | Off |

## Expand and resize

Click **Expand panel** next to the tabs to let the right pane take the chat's space. Click **Collapse panel**, or close the pane, to bring the chat back.

Drag the pane's left edge to resize it. Double-click the edge to return to the default layout.

Closing the pane while a device tab is active moves that device to a floating preview over the chat. See [Device hub](zeron://docs/device-hub).
