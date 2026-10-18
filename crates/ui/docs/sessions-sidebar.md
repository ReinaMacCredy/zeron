Every chat is a session in the sidebar. The sidebar shows what each agent is doing and lets you pin, rename, group, and archive sessions.

## Status

Each row shows the session's status in its top-right corner:

| Status | Meaning |
| --- | --- |
| **Working** (blue, with a timer) | The agent is running. |
| **Input** | The agent is waiting for you: a question or an approval. |
| **Failed** | The run ended with an error. |
| **Done** | The run finished. |

An idle session shows how long ago it was active instead. **Done** and **Input** titles stay bright until you open them.

## Row actions

Hover a row to get its quick buttons: **Pin chat** and **Archive session**. Hover longer and a card shows the session's project, device, branch, agent and model, and any pull request.

Right-click a row for the full menu: **Rename**, **Pin** or **Unpin**, **Archive**, **Copy** (the path, a Zeron conversation link, or the agent's session ID), and **Delete…**, which removes the session for good after you confirm.

## Renaming

Double-click a row, or choose **Rename**. Type the new title, then press Enter or click away to save. Escape cancels.

## Pinning

Pinned sessions sit in a **Pinned** group at the top. Pin from the hover button or the right-click menu, or drag a row into the group. Drag pinned rows to reorder them.

## Archiving

Archive with the hover button, the right-click menu, or Cmd+Shift+A for the open session. Archived sessions move to an **Archived** group at the bottom of the sidebar; hover one and click **Unarchive** to bring it back.

[Settings > Archived sessions](zeron://settings/archived) lists everything archived, with an **Unarchive** button on each row and **Delete all** to clear them permanently.

## Finding and grouping

**Search** at the top of the sidebar opens the [command palette](zeron://docs/command-palette), which searches every thread, archived ones included. Hold Cmd to see number hints on the first nine rows, and press Cmd+1 to Cmd+9 to open one.

The **…** button at the sidebar's top edge (**Sidebar view options**) changes the layout:

- **Organize**: **By device**, **By project**, **None**, or **Inbox**.
- **Sort**: **Last updated**, **Created**, or **Active first**.
- **Show**: which details each row carries, such as **Branch**, **Pull request**, and **Harness**, plus the **Project filter** and the **Archived** group.
- **Compact**: one-line rows with a colored dot for status instead of cards.
- **Create Section**: a named group you fill by dragging sessions in.

## Inbox shelves

With **Organize** set to **Inbox**, the list splits into shelves: **Inbox** for open sessions, **Working** for running ones (when **Show > Working** is on), and **Settled** for sessions you are done with. Click a shelf's header to collapse it. A session with no activity for three days settles on its own; choose **Settle** from the hover button or right-click menu to do it now.

> Unsent drafts wait in a **Drafts** group under Pinned. See [Start a new chat](zeron://docs/new-chat).
