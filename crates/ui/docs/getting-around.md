The Zeron window has three parts: the app rail on the far left, the sidebar with your sessions, and the main column where the open chat or page lives.

![The Zeron window: app rail, sidebar, and an open chat](getting-around-window.jpg)

## The app rail

The narrow strip on the left switches between Zeron's pages. Hover a button to see its name.

| Button | What it opens |
| --- | --- |
| **Home** | Back to your chats. |
| **Inbox** | The [Inbox](zeron://inbox), with what needs you today. A number shows how many agents are waiting for your input. |
| **Tasks** | The [Tasks](zeron://tasks) page. A number shows how many tasks need you. |
| **Supervisor** | The [Supervisor](zeron://supervisor) page. A number counts its open **Needs you** cards. |
| **Pull requests** | The [Pull requests](zeron://pull-requests) board. |
| **Usage** | The [Usage](zeron://usage) page. |

Further down the rail:

- **Usage rings** show how much of your plan's limits are left, one ring per agent account, at most two. The outer ring is the week and the inner ring the current session. Hover for details. Click a ring to list that provider's accounts, with the one in use marked **In use**; click another to switch, or click **Open Usage**.
- A microphone button for voice mode, when Codex is installed. See [Voice and dictation](zeron://docs/voice).
- Your **account** button, with your initial. Its menu starts with your name and email (or **Local** when you are not signed in), then a **Usage · this device** section with each provider's plan and meters (**Refresh** updates them), **Usage** to open the Usage page, and one sync row: **Enable sync**, **Sync setup in progress**, **Finish sync setup**, or **Sign out**. On Windows and Linux it also has **Check for updates**. To sync with your other devices, see [Devices](zeron://docs/devices).
- **Help**: recent releases, the full changelog, keybindings, and these docs. See [Updates and what's new](zeron://docs/updates).
- A download button, when an app update is available.
- **Settings** at the bottom. Cmd+, opens and closes it too.

## The sidebar

The sidebar lists your sessions, drafts, and projects. Its header has **Search** (which opens the [command palette](zeron://docs/command-palette)), a project filter, **New project**, and **New session**. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

Drag the sidebar's right edge to resize it, from 242 to 400 px wide. Double-click the edge to return to the default width.

To hide or show the sidebar, click the sidebar button at the top left of the window or press Cmd+B (**Toggle left sidebar**). Zeron remembers your choice.

With the sidebar hidden, move the pointer just right of the app rail and it slides out as a floating panel. Move away and it hides again; press Escape to dismiss it. Turn this off with **Reveal sidebar on hover** in [Settings > Appearance](zeron://settings/appearance).

## Going back and forward

Zeron keeps a history of where you have been: chats, drafts, Settings pages, Inbox, Tasks, Supervisor, Usage, pull requests, and these docs. To walk it:

- Click the **Back** and **Forward** arrows in the titlebar.
- Press the back and forward buttons on your mouse.
- Swipe left or right with two fingers on the trackpad.

While you swipe, a round pill slides in from the edge of the window and fills as you go. Once it is far enough it changes color and names the page you will land on; let go to switch. Let go early and the page springs back. With nowhere to go, the pill reads **No previous page** or **No next page**.

On a trackpad that supports it, you feel a light tick when the swipe is far enough to switch.

> Swiping follows the macOS trackpad setting **Swipe between pages**. If swiping does nothing, check that it is turned on in System Settings.

## Session tabs

The titlebar shows a tab for each session open on this device. A tab shows the agent's mark (which changes while the agent works), the title, and a status dot that turns into a close button when you hover the tab or select it.

![Session tabs in the titlebar with the tab menu open](getting-around-tabs.jpg)

Selecting a session anywhere (the sidebar, the command palette, a notification) opens its tab. A chat you started opens its tab again by itself when it begins a new run, and any chat that starts waiting for your input opens its tab too.

Closing a tab only hides it here. The session keeps running and stays in the sidebar, where a click opens it again.

- Click a tab to switch to it.
- Middle-click a tab, or click its close button, to close it.
- Drag a tab to reorder it, or drag it onto the chat to open a split. See [Split view](zeron://docs/split-view).
- Right-click a tab for **Close tab**, **Close others**, **Close to the right**, **Close to the left**, and **Close idle tabs**. **Close idle tabs** closes idle sessions except the one you are viewing, the sessions in a split, and drafts.

Cmd+W closes the active tab in the right pane first, then the session tab you are on. With neither left, it closes the window. Click Zeron's Dock icon to open the window again.

## These docs

Choose **Docs** in the rail's **Help** menu to open these pages. **Overview** shows every article as a card, grouped by section. The list on the left has a search field that matches article titles and text.

The buttons under an article's title open the page or setting it describes. **Previous** and **Next** at the bottom step through the articles in order.

## The right pane

The open chat can show a right pane with changes, files, a browser, and a terminal. Toggle it with Cmd+R (**Toggle right sidebar**). See [Right pane](zeron://docs/right-pane).
