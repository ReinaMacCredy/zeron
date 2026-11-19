Zeron has real terminals in two places: a dock under each chat, and full-window terminal tabs in the titlebar. The right pane has no terminal tab.

## The terminal dock

Press Cmd+J, click the terminal button at the far right of the session footer (tooltip **Terminal**), or type `/terminal` to open the dock under the chat. Cmd+J works on New chat too. Press Cmd+J again to close it and return to the composer; `/terminal` only opens or focuses it. The footer button stays highlighted while the dock is open. On a page with no chat, such as Inbox or Settings, Cmd+J shows **The terminal opens from a chat** instead.

![The terminal dock under a chat](terminal-drawer.jpg)

Each chat keeps its own dock tabs, and they keep running when you switch chats. The first tab opens in the chat's folder.

- Click **+** (tooltip **New terminal**) for another tab.
- Click a tab's close button (tooltip **Close terminal**) or middle-click it to end it. Closing the last tab closes the dock.
- Click the chevron (tooltip **Hide terminal**) to hide the dock without ending anything.
- Drag the top edge to change the height, and double-click it to reset.

The dock runs the full width under the chat and the **Live state** column. Each project Action you run from the Live state column opens its own dock tab, named after the action. See [Projects](zeron://docs/projects).

## Terminal tabs

A terminal tab fills the window like a session. Open one from the Cmd+K palette with **New terminal**, **New Claude Code CLI**, or **New Codex CLI**. The last two start `claude` or `codex` directly. With the sidebar collapsed, the **+** (tooltip **New tab**) in the titlebar offers the same rows.

Terminal tabs start in the selected chat's folder, or the selected project, or your home folder. Open terminals are listed under **Terminals** in the sidebar.

- Closing a tab in the titlebar, or pressing Cmd+W, hides it but keeps the process running. Click its row under **Terminals** to bring it back.
- The close button on the sidebar row (tooltip **Close terminal**) ends the process.

## Copy, paste, and quoting

Select text with the mouse, then press Cmd+C to copy; Ctrl+C still interrupts the program. Cmd+V pastes. When you finish a drag selection, Zeron also stages the text as a quote in the chat's composer, so you can ask the agent about it.

## Terminal background

In [Settings > Appearance](zeron://settings/appearance), under **Advanced look** > **Material and background**, **Terminal background** sets the artwork behind terminal tabs: **Same as chat**, **Custom**, or **Off**. The row shows only after you set a **New thread background** image.

> Rebind **Toggle terminal** in [Settings > General > Shortcuts](zeron://settings/shortcuts).
