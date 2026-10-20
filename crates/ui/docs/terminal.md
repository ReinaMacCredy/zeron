Zeron has real terminals in three places: a drawer under each chat, full-window terminal tabs in the titlebar, and a Terminal tab in the right pane.

## The terminal drawer

Press Cmd+J, or click the terminal button at the far right of the session footer (tooltip **Terminal**), to open the drawer under the chat. Press Cmd+J again to close it and return to the composer. The footer button stays highlighted while the drawer is open.

![The terminal drawer under a chat](terminal-drawer.jpg)

Each chat keeps its own drawer tabs, and they keep running when you switch chats. The first tab opens in the chat's folder.

- Click **+** (tooltip **New terminal**) for another tab.
- Click a tab's close button (tooltip **Close terminal**) or middle-click it to end it. Closing the last tab closes the drawer.
- Click the chevron (tooltip **Hide terminal**) to hide the drawer without ending anything.
- Drag the top edge to change the height, and double-click it to reset.

The drawer runs the full width under the chat and the **Live state** column. Project Actions run in new drawer tabs, too. See [Projects](zeron://docs/projects).

## Terminal tabs

A terminal tab fills the window like a session. Open one from the Cmd+K palette with **New terminal**, **New Claude Code CLI**, or **New Codex CLI**. The last two start `claude` or `codex` directly. With the sidebar collapsed, the **+** (tooltip **New tab**) in the titlebar offers the same rows.

Terminal tabs start in the selected chat's folder, or the selected project, or your home folder. Open terminals are listed under **Terminals** in the sidebar.

- Closing a tab in the titlebar, or pressing Cmd+W, hides it but keeps the process running. Click its row under **Terminals** to bring it back.
- The close button on the sidebar row (tooltip **Close terminal**) ends the process.

## Terminal in the right pane

Click **Terminal** in an empty right pane, or click **+** and choose **Terminal**. Each click opens a new terminal tab for this chat, in the chat's folder and on the chat's device. The **Terminal** row in the [Live state](zeron://docs/live-state) column shows the first one, or opens one when there is none.

It is the same terminal as the drawer's, but it lives by the right pane's rules:

- It has no tab bar of its own: no **+**, **Close terminal**, or **Hide terminal**. Its tabs sit in the right pane's strip with the pane's other tabs. See [Right pane](zeron://docs/right-pane).
- Closing its tab (the ×, a middle-click, **Close tab**, or Cmd+W) always ends the process. Hiding the right pane keeps it running.
- Cmd+J does not open or close it.
- The tab is named after the title the shell sets, or **Terminal 1**, **Terminal 2**, and so on.
- When the program exits, the tab stays open with a `[process exited N]` line until you close it.

## Copy, paste, and quoting

Select text with the mouse, then press Cmd+C to copy; Ctrl+C still interrupts the program. Cmd+V pastes. When you finish a drag selection, Zeron also stages the text as a quote in the chat's composer, so you can ask the agent about it.

## Terminal background

In [Settings > Appearance](zeron://settings/appearance), under **Material and background**, **Terminal background** sets the artwork behind terminal tabs: **Same as chat**, **Custom**, or **Off**. The row shows only after you set a **New thread background** image.

> Rebind **Toggle terminal** in [Settings > Shortcuts](zeron://settings/shortcuts).
