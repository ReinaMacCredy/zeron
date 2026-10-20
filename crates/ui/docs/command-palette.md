The command palette finds threads, files, and commands from one search box. The session switcher flips between recent sessions without touching the mouse.

## Open the palette

Press Cmd+K, or click **Search** at the top of the sidebar. Press Cmd+K again, press Escape, or click outside to close it. The search box reads **Search threads, commands and files…**.

Use the arrow keys to move, Enter to open the highlighted row, and Escape to close.

## Tabs

The palette has four tabs. Press Cmd+1 to Cmd+4 to jump to one, or Tab and Shift+Tab to step through them.

| Tab | Before you type | After you type |
| --- | --- | --- |
| **All** | **Recent Files**, **Quick Actions**, **Recent Threads** | Matching **Threads**, **Commands**, **Files**, and **In Files** |
| **Threads** | **Recent Threads** | Threads matching the title, project, branch, device, or pull request |
| **Commands** | **Quick Actions** | Matching **Commands** |
| **Files** | **Recent Files** | Matching **Files** and lines **In Files** |

Thread search includes archived threads and ignores the sidebar's project filter. File search covers the workspace of the chat you have open; type at least two characters.

## Commands

The commands include **New chat**, **New terminal**, **New project**, **Open settings**, **Open inbox**, **Open usage**, **Tasks**, a light or dark theme switch, and **Archive thread** for the open chat. Each row shows its shortcut when it has one. In a chat, the palette also lists the project's actions, plus **Add action…** to create one.

Opening a thread switches to it. Opening a file shows it in the right pane; a content match opens the file at that line.

> Drag a thread row out of the palette to drop it where a sidebar row can go, such as a split side or a composer to mention it. If the drop misses, the palette comes back.

## Session switcher

Hold Ctrl and press Tab to open the switcher: a row of cards for your sessions, most recently used first. The first press lands on the previous session. Keep holding Ctrl and press Tab again to move along (add Shift to go back), then release Ctrl to open the highlighted session. Press Escape to cancel, or click a card.

Each card shows the last view of that session, its title, project and device, and a status: **Working**, **Input**, **Failed**, or **Done**. When the right pane has focus, Ctrl+Tab cycles the right pane's tabs instead.

[Settings > General](zeron://settings/general) controls the switcher:

- **Session switcher shows**: every session, or only those active within a window from **1h** to **7d**. Sessions open as tabs always appear, whatever the window.
- **Session switcher shows at most**: cap the number of cards at **All** or anywhere from 2 to 50.
- **Ctrl+Tab cycles tabs**: move straight to the next tab instead of opening the switcher.

## Type to focus

When no text field has focus, you can just start typing: the characters go into the composer. Cmd+V pastes there too. This works after clicking the transcript, a sidebar control, or the app rail.
