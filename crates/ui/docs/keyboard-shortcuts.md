Most of Zeron can be driven from the keyboard. Rebindable shortcuts live in [Settings > Shortcuts](zeron://settings/shortcuts); you can also reach that page from the Help menu in the app rail with **Keybindings**.

## The most useful defaults

| Shortcut | What it does |
| --- | --- |
| Cmd+K | Open or close the command palette |
| Cmd+N | New session (a new chat canvas) |
| Cmd+Shift+N | New project |
| Cmd+B | Toggle left sidebar |
| Cmd+R | Toggle right sidebar |
| Cmd+E | Toggle files panel |
| Cmd+J | Toggle terminal |
| Cmd+/ | Open model picker |
| Cmd+; | Toggle sidechat (btw) |
| Ctrl+Tab, Ctrl+Shift+Tab | Next or previous session (or right pane tab) |
| Cmd+1 to Cmd+9 | Jump to session 1 to 9 in the sidebar |
| Cmd+Shift+A | Archive session |
| Cmd+S | Save file |
| Cmd+Shift+R | Reload browser page |
| Cmd+U | Random wallpaper |
| Cmd+, | Open or close Settings |

Cmd+K and Cmd+, are fixed; you can change the rest.

Two shortcuts live on their feature's own settings page instead: **Hold to dictate** (Cmd+D) in **Settings > Voice**, and **Capture Appshot** in **Settings > Appshots**.

## Rebinding a shortcut

The **Keyboard shortcuts** page groups rows under **Appearance**, **Files**, **Browser**, **Panels**, **Sessions**, **Projects**, and **Jump to session**.

![Rebinding a shortcut](shortcuts-recording.jpg)

1. Click the binding on the right of a row. It changes to **Press keys…**.
2. Press the new combination.
3. Press Escape to cancel instead.

Shortcuts must be unique. If the combination already belongs to another shortcut, Zeron keeps the old binding and says which shortcut owns it. Cmd+Enter is reserved for the composer. **Capture Appshot** has one extra rule: it needs Control, Option or Command with a letter, number, function key or navigation key. A changed row shows **Reset** to go back to its default. **Restore defaults** at the top resets every shortcut.

> **Restore defaults** also resets **Hold to dictate**, **Capture Appshot**, and the keyboard options in Settings > Chat and Settings > General: **Send messages with**, **Follow-up behavior**, **Stop active agent with Escape**, **Thinking level shortcut**, and the session switcher settings.

## Related keyboard settings

These live in [Settings > Chat](zeron://settings/chat) and [Settings > General](zeron://settings/general):

- **Send messages with** (Chat): Enter or Cmd+Enter.
- **Thinking level shortcut** (macOS only): press Cmd+- or Cmd+= to lower or raise the thinking level, or switch the modifier to **⌃ Control**. On other systems the keys are Ctrl+- and Ctrl+=.
- **Stop active agent with Escape** (Chat): press Escape to stop the running agent when no dialog or menu is open. Off by default.
- **Ctrl+Tab cycles tabs**: move straight to the next tab instead of opening the session switcher.

## Keys in other places

- Cmd+W (macOS) closes the right pane's active tab first, then the focused session tab, and closes the window only when neither is left. In the browser pane it closes the browser tab.
- Browser: with focus in the [browser](zeron://docs/browser) pane, Cmd+L focuses the address bar, Cmd+T opens a tab, Cmd+W closes it, and Cmd+[ and Cmd+] go back and forward. A key you bound to another shortcut keeps that shortcut.
- Device screen: with focus on a [Device hub](zeron://docs/device-hub) screen, Cmd+R is sent to the device instead of toggling the right sidebar.

## Text editing keys

The composer uses the usual macOS editing keys:

| Keys | What they do |
| --- | --- |
| Cmd+Left, Cmd+Right | Start or end of the line |
| Cmd+Up, Cmd+Down | Start or end of the text |
| Option+Left, Option+Right | Back or forward one word |
| Add Shift to any of these | Select as you move |
| Cmd+Backspace, Cmd+Delete | Delete to the start or end of the line |
| Option+Backspace, Option+Delete | Delete the word before or after the cursor |
| Cmd+Z, Shift+Cmd+Z | Undo, redo |
| Cmd+A, Cmd+C, Cmd+X, Cmd+V | Select all, copy, cut, paste |

On Windows and Linux, Home and End move to the line's start or end, and Ctrl takes the place of Option for word moves and of Cmd for undo and the clipboard.

## The menu bar

On macOS the menu bar has three menus:

| Menu | Items |
| --- | --- |
| **Zeron** | **About Zeron**, **Check for Updates…**, **What’s New…**, **Settings** (Cmd+,), **Services**, **Hide Zeron** (Cmd+H), **Hide Others** (Option+Cmd+H), **Show All**, **Quit Zeron** (Cmd+Q) |
| **Edit** | **Undo**, **Redo**, **Cut**, **Copy**, **Paste**, **Select All** |
| **Window** | **Minimize** (Cmd+M), **Zoom**, **Close Window** (Cmd+W), then your open windows |

On Windows and Linux there is no **Window** menu, and **Check for updates** is in [Settings > Devices & updates > Updates](zeron://settings/updates). See [Updates and what's new](zeron://docs/updates).
