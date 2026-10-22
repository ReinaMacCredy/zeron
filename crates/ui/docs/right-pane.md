The right side of the window holds two columns next to the chat: the right pane, which shows tabs such as a browser, a terminal, or a file, and the Files panel, which shows the chat's file tree.

## Open and close

| Column | Titlebar button | Shortcut |
| --- | --- | --- |
| Right pane | **Toggle right sidebar** (rightmost) | Cmd+R |
| Files panel | **Show files panel** / **Hide files panel** | Cmd+E |

Each toggle works on its own column only: Cmd+R never hides the file tree. Cmd+R also works on the Pull requests page. The Files panel is not available on New chat. Rebind them as **Toggle right sidebar** and **Toggle files panel** in [Settings > Shortcuts](zeron://settings/shortcuts).

While this chat's subagents run, the Files button shows how many, because they are listed in the Files panel. You can find them there even while the panel is closed.

## Add a surface

An empty right pane lists what it can show. Once a tab is open, click **+** (tooltip **New tab**) for the same list, plus **Files**, which docks the Files panel.

![The right pane's + menu](right-pane-menu.jpg)

| Row | Opens |
| --- | --- |
| **Browser** | A web browser tab. See [Browser](zeron://docs/browser). |
| **Terminal** | A terminal tab for this chat. See [Terminal](zeron://docs/terminal). |
| **Device** | A device screen. See [Device hub](zeron://docs/device-hub). |
| **New sidechat** | A `/btw` sidechat. See [Side chat](zeron://docs/side-chat). |
| **Diffs**, **History** | The chat's changes and commit history. See **Diffs and History** below. |

**Device** is always listed, but it is dimmed when it cannot open. Hover it for the reason: **Devices are only available from a thread.**, **Devices are only available on the chat's own computer.**, or **This Zeron engine does not support devices.**

**Diffs** and **History** show only in a chat whose project is a git repository, so never on New chat. **New sidechat** does not show when the chat is itself a sidechat.

## Tabs

Tabs belong to each chat: switching chats shows that chat's own tabs. They last until Zeron quits. A fresh launch starts with the pane closed and no tabs. Browser logins are kept separately; see [Browser](zeron://docs/browser).

- Hover a tab for its full file path, page address, or device details.
- A spinner replaces the tab's icon while a side chat is working, a page is loading, or a subagent is streaming.
- A dot on a file tab means unsaved edits. Hover it to get the close button.
- Drag tabs to reorder them, and middle-click to close one. Drag a file tab onto the composer to attach that file.
- After you close a tab, the tab you used most recently before it becomes active. Closing the last tab closes the pane; a docked Files panel stays open on its own.

Right-click a tab for **Close tab**, **Close other tabs**, **Close tabs to the left**, and **Close tabs to the right**. On a side chat or delegated child tab, the menu also has **Rename** and **Delete…** for that chat, plus **Add plan** on a delegated child that can take one. On a device tab it adds **Rename**.

Cmd+W closes the active pane tab first. When the pane has focus, Ctrl+Tab and Ctrl+Shift+Tab move between its tabs.

## Diffs and History

These tabs show the chat's own checkout. The Git tab on the Pull requests page is a separate view with staging and commits; see [Git](zeron://docs/git).

### Diffs

Every click on **Diffs** opens a new tab, so you can keep several scopes side by side. The menu at the left of the header picks the scope, and the tab is named after it:

| Scope | Shows |
| --- | --- |
| **Working tree** (default) | Uncommitted changes, updated live |
| **Branch changes** | Everything the branch adds over a base branch, uncommitted changes included. Click the base branch name to pick another; type in **Search branches…** to filter. |
| **Latest turn** | Changes since this chat's last turn started |

The line under the header counts the changed files and added and removed lines. **Partial snapshot** means the diff was too large and was cut short.

- **Split view** shows old and new side by side; **Wrap long lines** wraps instead of scrolling sideways. Both are remembered and apply to every Diffs tab and the pull request Code view.
- **Collapse all files** folds every file (**Expand all files** opens them again). Click a file's header to fold just that file.
- The document button on a file's header (**Open in file browser**) opens the file in an editor tab.
- **Discard changes** (Working tree only) asks **Discard working tree changes?** and then puts tracked files back to the last commit and deletes untracked files. Ignored files are kept. This cannot be undone. The button is dimmed when there is nothing to discard or the diff is a partial snapshot.

There is no staging here; stage and commit from the [Git](zeron://docs/git) tab.

Click a file name on a tool row in the transcript: if that file has uncommitted changes, a **Working tree** tab opens scrolled to it; otherwise the file opens in an editor tab.

### Comment on a line

Hover a line and click the **+** in the line numbers. Type in **Request a change…** and click **Comment** or press Enter (Shift+Enter adds a line, Escape cancels). In split view only the new (right) side takes comments.

![Reviewing changes in the Diffs tab](right-pane-diffs.jpg)

The comment stays on its line, with an edit button and **Remove comment** on hover. Comments belong to the chat, so they show in every Diffs tab where that line appears.

Above the composer, a **1 comment** (or **N comments**) pill collects them. Click it to see the list, remove one with its ×, or **Clear all**. Your next message carries them: they are added under your text as a list of `file:line` notes, where L means a line in the original file and R a line in the changed one. With no text of your own, the message reads "Address the review comments below." In the transcript the message shows an **N comments** pill; hover it to read them.

Findings from a [code review](zeron://docs/review) appear on their lines in the same way, with **Fix** and **Dismiss**.

### History

**History** shows the commit graph of the chat's checkout. The header shows the branch, the number of commits, and how far it is ahead of or behind its base.

- The magnifier (**Search commits**) finds a commit by SHA or subject.
- **Fetch all** fetches from every remote. The fold button switches between **Show branch tips** and **Show all commits**. Click **Refresh** to pick up new commits.
- Click a commit to open it as its own Diffs tab, named after its subject. You can comment on its lines too.
- Click a SHA to copy the full SHA; it reads **Copied**.
- Hover the column headers for **Columns** to show or hide **Author**, **Date**, and **SHA**. Drag a header to reorder, or a divider to resize. Right-click **Author** and choose **Name** to show names instead of avatars.
- **Load more** loads the next 100 commits.

## Files panel

Type in **Search files** to find a file or folder by name, then use Up and Down and press Enter to open it. Escape clears the search. Click a file to open it in a tab. Shift-click selects a range and Cmd-click adds or removes one item.

The eye button (**Show all files (even hidden)**) also lists files that git ignores. Files whose names start with a dot always show, and `.git` never does. It is the same switch as **Show hidden and ignored files** in [Settings > Files](zeron://settings/files).

Right-click for **Add to chat**, **Copy path**, **Rename…**, and **Delete…**. Delete is permanent and asks first. Drag rows onto a folder to move them, or into the chat to attach them.

### Subagents and Chats

Under the tree, two sections list what this chat has started:

- **Subagents**: the agent's built-in subagents, running ones first. Finished ones fold under **Finished**. Click a row to read its transcript in the right pane.
- **Chats**: side chats, forks, and delegated children of this chat, newest activity first. Click a row to open it in the right pane, double-click to rename it, or right-click for its menu.

Hover the **Chats** header for **New side chat** (an empty chat that starts fresh beside this one) and **Fork this chat** (a copy of this conversation that carries on by itself). See [Side chat](zeron://docs/side-chat). Each section shows 10 rows, then **Show N more**.

## Edit files

Press Cmd+S to save. Markdown files open as a preview; click **Show Markdown code** to edit the source. The header also has **Reveal file in tree**, **Open in default editor** (for files on this computer), and a word wrap toggle.

You can edit UTF-8 text files up to 1 MB inside the chat's folder. Some files open read-only or not at all, with a line saying why:

- Text files from 1 to 8 MB, files with mixed line endings, and symlinks are read-only.
- A file opened from a link outside the chat's folder shows **Read-only: outside this chat's folder.**
- Files over 8 MB, binary files, and text that is not UTF-8 do not open.
- Images open in a viewer. Pinch or Ctrl+scroll to zoom.

Closing a tab with unsaved edits, or quitting, saves it first. If the file changed on disk while you were editing, a bar reads **This file changed outside Zeron.** with **Keep Editing** and **Reload from Disk**.

[Settings > Files](zeron://settings/files) has:

| Setting | Default |
| --- | --- |
| **Autosave**: save a short time after your last edit | Off |
| **Autosave delay**: **300 ms**, **600 ms**, **900 ms**, **1.5 s**, or **3 s** (shown while Autosave is on) | 900 ms |
| **Word wrap**: the same switch as the header toggle | Off |
| **Show hidden and ignored files**: the same switch as the eye button | Off |

## Expand and resize

Click **Expand panel** next to the tabs to let the right pane take the chat's space. Click **Collapse panel**, or close the pane, to bring the chat back.

Drag the left edge of either column to resize it. Double-click the edge to return to the default layout.

When the window is too narrow for the sidebar, the chat, the right pane, and the Files panel together, the column you opened longest ago hides. It keeps its tabs and comes back when there is room. Toggling a hidden column brings it forward instead of closing it.

Closing the pane while a device tab is active moves that device to a floating preview over the chat. See [Device hub](zeron://docs/device-hub).
