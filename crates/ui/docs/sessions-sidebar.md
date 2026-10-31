Every chat is a session in the sidebar. The sidebar shows what each agent is doing and lets you pin, rename, group, and archive sessions.

## Status

Each row shows the session's status in its top-right corner:

| Status | Meaning |
| --- | --- |
| **Working** (blue, with a timer) | The agent is running. |
| **Asking** | The agent is waiting for you: a question or an approval. With several questions it counts them, such as **Asking 1/2**. |
| **Plan ready** | The agent wrote a plan that waits for your approval. |
| **Failed** | The run ended with an error, or your message was never delivered. |
| **Done** | The run finished. |
| **Queued** (amber) | Your message is waiting to be delivered. |
| **Expired** | A side chat older than 24 hours. |

An idle session shows how long ago it was active instead. While the agent runs background work, **Working** gives way to a count such as "2 subagents · 1 shell". **Done** and **Asking** titles stay bright until you open them.

A card row carries more than the title:

- Above the title, the location: the project, or the device for a chat without one (when **Show > Location** is on).
- Below the title, the branch, after a worktree icon when the chat runs in a worktree, then the chat's changes such as **+7 −1** once Review has counted them. A side chat, a fork, or a chat that another chat opened has no row of its own while the chat it came from has one: it is listed in that chat's **Live state** column under **Beside this thread**, and opens as a tab in its right pane. When it does have a row, a chat that another chat opened shows "↳" and the name of the chat that opened it instead (see [Agents and other threads](zeron://docs/threads)).
- On the right of that line: the pull request badge, a bot icon with the number of subagents, and a monitor icon when the chat runs on another device.
- An amber pen and a faint amber wash mean the chat has unsent text in its composer.

## Row actions

Hover a row to get its quick buttons: **Pin chat** and **Archive session**, plus **Settle** in Inbox mode. Hover longer and a card shows the session's project, device, branch, agent and model, which chat opened it, how many subagents are running, and any pull request. **Compact** rows have neither the **Pin chat** button nor the card: hovering shows only the archive button in place of the time.

Right-click a row for the full menu: **Rename**, **Add plan** (on a chat of this computer that has no plan yet), **Pin** or **Unpin**, **Settle** or **Unsettle** (in Inbox mode, on a row that is not pinned, not in a section, and not working or waiting on you), **Archive**, **Copy**, and **Delete…**, which removes the session for good after you confirm. A Task lead's row also has **Collapse step threads** or **Expand step threads**. A chat that another chat opened gets only **Rename** and **Delete…**, plus **Add plan** when it is not a side chat.

**Copy** has:

- **Path**: the chat's folder, when it is known.
- **Zeron conversation link**: a `zeron://` link that opens this chat in Zeron. Its format is in [Command line and servers](zeron://docs/command-line).
- **Codex conversation link**: for Codex chats, a link that opens the conversation in the Codex app.
- **Harness session ID**: the agent's own session ID.

## Step threads

The lead of a Task with a plan (see [Agent trees and plans](zeron://docs/mission-control)) is one row with a progress ring and the step it is on, such as **Task · step 4 of 7**, or **Task · done** once every step is done; click it to open the Task's page. Only the step threads that are running or asking you show indented under it, and a step that asks reads **Asking**. One faint row, such as **5 more steps**, counts the rest; click it to open the Task's page.

![A Task in the sidebar: the lead with its ring, the step it is on, and 3 more steps](sidebar-mission.jpg)

Hover the lead and click the chevron at its left edge, Tab to the chevron and press Space, or use the right-click menu, to fold the steps away. A collapsed lead keeps the chevron showing and sums up its steps, for example **4 · 1 running**: the number of steps, then the most pressing of asking you, failed, or running. When a step needs you, an attention dot appears on the lead. Opening a step, from the Task's page or a link, unfolds its lead again. If you fold a lead while one of its steps is open, the lead row stays shaded to show it holds the open chat. Zeron remembers which leads you collapsed.

## Renaming

Double-click a row, or choose **Rename**. Type the new title, then press Enter or click away to save. Escape cancels.

## Pinning

Pinned sessions sit in a **Pinned** group at the top. Pin from the hover button or the right-click menu, or drag a row into the group. Drag pinned rows to reorder them, or drag one out to unpin it. You can pin up to 200 sessions. The **Pinned** header shows the count; click it to collapse the group.

## Archiving

Archive with the hover button, the right-click menu, or Cmd+Shift+A for the open session. Archived sessions move to an **Archived** group at the bottom of the sidebar; hover one and click **Unarchive** to bring it back. The group shows ten rows, then a "Show N more" row loads 25 at a time. It is hidden when **Show > Archived** is off or in Inbox mode.

[Settings > General > Archived](zeron://settings/archived) lists everything archived, with an **Unarchive** button on each row and **Delete all** to clear them permanently.

## Finding and grouping

**Search** at the top of the sidebar opens the [command palette](zeron://docs/command-palette), which searches every thread, archived ones included. Hold Cmd to see number hints on the first nine rows, and press Cmd+1 to Cmd+9 to open one.

The **…** button at the sidebar's top edge (**Sidebar view options**) changes the layout:

- **Organize**: **By device**, **By project**, **None**, or **Inbox**. The default, **None**, lists every session under one **Sessions** header; collapsing that header also takes its rows out of Cmd+1 to Cmd+9. **By device** puts this device's group first. **By project** gives each repository one group, even when it is checked out on several devices, and puts sessions without a project in **Chats** at the top.
- **Sort**: **Last updated** (the default) or **Created**. **Active first** is a separate check that combines with either and lifts every session showing a status (**Working**, **Asking**, **Failed**, or **Done**) above idle ones; it is off by default.
- **Show**: checkmarks for **Branch**, **Pull request**, **Harness**, **Project icon**, **Location**, **Project filter**, **Working**, and **Archived**. All are on by default except **Working**, which only applies in Inbox mode. Turning off **Pull request** also stops Zeron from looking up pull requests.
- **Compact**: one-line rows with a status glyph instead of cards. Off by default.
- **Create Section**: a named group you fill yourself. See Sections below.

![Sidebar view options](sidebar-view-options.jpg)

In Inbox mode, **Sort** and **Show > Archived** do nothing.

### The project filter

The filter button in the sidebar header (tooltip **Filter by project**) opens a list of your projects, with **Search projects…** at the top, **All projects** first, and **New project** at the bottom. A project with no reachable device shows a crossed-out wifi glyph. Pick a project and the session list, **Drafts**, and **Archived** show only its sessions, **By project** grouping turns into one flat list, and the button's tooltip reads **Filter:** and the project name. **Search** then reads "Search" and the project name, but the palette still searches every thread. Pick **All projects** to clear the filter; hiding the button with **Show > Project filter** clears it too. Right-click a project in the list to rename or remove it (see [Projects and worktrees](zeron://docs/projects)).

## Sections

Sections are named groups you build by hand. They sit under Pinned and Drafts, above the rest of your sessions.

1. Open **Sidebar view options** and click **Create Section**.
2. In the **New section** dialog, type a name (**Section name**, up to 120 characters) and click **Create section**.
3. Drag sessions onto the section's header or into it. An empty section says "Drop sessions here".

Sessions in a section keep the order you put them in, not activity order. Click the header to collapse the section. Hover the header and click its **…** (**Section options**) for:

- **Edit section**: rename it, then **Save**.
- **Archive all**: archive every session in it.
- **Delete**: remove the section. Its sessions go back to the regular list.

With two or more sections, drag a section's header to move it above or below the others.

Sections and pins follow your account in a synced workspace, and stay on this device in a local one.

## Inbox shelves

With **Organize** set to **Inbox**, the list splits into shelves: **Inbox** for open sessions, **Working** for running ones (when **Show > Working** is on), and **Settled** for sessions you are done with. Click a shelf's header to collapse it. A session with no activity for three days settles on its own; choose **Settle** from the hover button or right-click menu to do it now. Sessions that are working or waiting for you never settle. On a settled row, the hover button **Un-settle** takes the place of Archive and brings it back to **Inbox**.

![Sidebar in Inbox mode with the Working and Settled shelves open](sidebar-inbox-shelves.jpg)

**Settled** rows are slim one-liners: the shelf shows ten, then a "Show N more" row loads 25 at a time. The **Archived** group is hidden in this mode.

> Unsent drafts wait in a **Drafts** group under Pinned. See [Start a new chat](zeron://docs/new-chat).
