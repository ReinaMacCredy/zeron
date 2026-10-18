The right side of the window holds two columns next to the chat: the right pane, which shows tabs such as a browser, a terminal, or a file, and the Files panel, which shows the chat's file tree.

## Open and close

| Column | Titlebar button | Shortcut |
| --- | --- | --- |
| Right pane | **Toggle right sidebar** (rightmost) | Cmd+R |
| Files panel | **Show files panel** / **Hide files panel** | Cmd+E |

Each toggle works on its own column only: Cmd+R never hides the file tree. The Files panel is not available on New chat. Rebind both in [Settings > Shortcuts](zeron://settings/shortcuts).

## Add a surface

An empty right pane lists what it can show. Once a tab is open, click **+** (tooltip **New tab**) for the same list, plus **Files**, which docks the Files panel.

| Row | Opens |
| --- | --- |
| **Browser** | A web browser tab. See [Browser](zeron://docs/browser). |
| **Terminal** | A terminal tab for this chat. See [Terminal](zeron://docs/terminal). |
| **Device** | A device screen. Only in a thread on the chat's own computer. |
| **New sidechat** | A side chat. See [Side chat](zeron://docs/side-chat). |
| **Diffs**, **History** | The chat's changes and commit history, in git projects. See [Git](zeron://docs/git). |

## Tabs

Drag tabs to reorder them, and middle-click to close one. Right-click a tab for **Close tab**, **Close other tabs**, **Close tabs to the left**, and **Close tabs to the right**. Cmd+W closes the active pane tab first. When the pane has focus, Ctrl+Tab and Ctrl+Shift+Tab move between its tabs.

## Files panel

Type in **Search files** to find a file, and click the eye button to show hidden and ignored files. Click a file to open it in a tab. Shift-click selects a range and Cmd-click adds one item.

Right-click for **Add to chat**, **Copy path**, **Rename…**, and **Delete…**. Delete is permanent and asks first. Drag rows onto a folder to move them, or into the chat to attach them.

## Edit files

File tabs are editable. Press Cmd+S to save. Markdown files open as a preview; click **Show Markdown code** to edit the source. The header also has **Reveal file in tree**, **Open in default editor**, and a word wrap toggle. [Settings > Files](zeron://settings/files) has **Autosave**, **Autosave delay**, **Word wrap**, and **Show hidden and ignored files**.

## Expand and resize

Click **Expand panel** next to the tabs to let the right pane take the chat's space. Click **Collapse panel**, or close the pane, to bring the chat back.

Drag the left edge of either column to resize it. Double-click the edge to return to the default layout.
