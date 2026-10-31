The Zeron window has three parts: the app rail on the far left, the sidebar with your sessions, and the main column where the open chat or page lives.

![The Zeron window: app rail, sidebar, and an open chat](getting-around-window.jpg)

## The app rail

The narrow strip on the left switches between Zeron's pages. Hover a button to see its name.

| Button | What it opens |
| --- | --- |
| **New chat** | [New chat](zeron://docs/new-chat), with a composer and the **Needs you**, **Running** and **Recent** cards. Cmd+N opens it from anywhere. |
| **Inbox** | The [Inbox](zeron://inbox): the history of what your agents asked, finished and failed today. |
| **Tasks** | The [Tasks](zeron://tasks) page. A number counts the tasks you captured and have not seen yet. |
| **Pull requests** | The [Pull requests](zeron://pull-requests) board. |

What needs you right now is not on the rail: the pill on the right of the titlebar counts it, and Cmd+I opens its tray. See **Needs you and running** below.

Further down the rail:

- The **usage ring** shows the tightest limit window of the agent accounts you use. Hover it for a card with each account's windows and when they reset, and click it to open the [Usage](zeron://usage) page. At a limit it turns red. See [Usage](zeron://docs/usage).
- A microphone button for voice mode, when Codex is installed and turned on in [Settings > Providers](zeron://settings/providers). See [Voice and dictation](zeron://docs/voice).
- **Help**: these docs, what's new, keyboard shortcuts, **Star Zeron on GitHub**, and **Agent updates** when an agent CLI has one. See [Updates and what's new](zeron://docs/updates).
- A download button, when an app update is available.
- **Settings** at the bottom. Cmd+, opens and closes it too. Settings opens as a panel over the app, with eight sections in a list on the left (General, Appearance, Notifications, Providers, Agents, Context & tools, This Mac, Devices & updates), then **Account**, and **Search settings** on top. A section with more than one page shows them as tabs above the page. Press / or Cmd+F to search: each found setting shows its path, such as **Appearance › Advanced look**, and picking one opens its page at that row. The header names the section, for example **Settings › Devices & updates**, and holds that page's actions. **Account** shows who you are signed in as, or **Stored on this device**, with **Enable sync** or **Sign out**. To sync with your other devices, see [Devices](zeron://docs/devices). Close it with the **×**, Escape, or a click outside the panel.

## The sidebar

The sidebar lists your sessions, drafts, and projects. Its header has **Search** (which opens the [command palette](zeron://docs/command-palette)), a project filter, **New project**, and **New session**. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

Drag the sidebar's right edge to resize it, from 242 to 400 px wide. Double-click the edge to return to the default width.

To hide or show the sidebar, click the sidebar button at the top left of the window or press Cmd+B (**Toggle left sidebar**). Zeron remembers your choice.

With the sidebar hidden, move the pointer just right of the app rail and it slides out as a floating panel. Move away and it hides again; press Escape to dismiss it. Turn this off with **Reveal sidebar on hover** in [Settings > Appearance](zeron://settings/appearance).

## Needs you and running

The pill on the right of the titlebar counts the threads that need you and the ones running, for example **2 need you · 1 running**. With nothing waiting it reads only the running count, and with nothing at all it is hidden.

Click it or press Cmd+I to open its tray:

![The Cmd+I tray: Supervisor on top, then Needs you with a question open in place](triage-tray.jpg)

- The top row, **Supervisor**, sums up what is going on and opens the [Supervisor](zeron://docs/supervisor) chat.
- **Needs you** lists each thread that asks a question, has a plan waiting for your OK, or failed. **Answer** and **Review** open the question or the plan right in the tray, so you answer without leaving the chat you are in; **Open** opens the thread.
- **Running** lists each thread that works, with its subagents under it.
- **History** opens the Inbox and **Tasks** opens Tasks.

Escape, a click outside or Cmd+I closes the tray. Change the shortcut in [Settings > General > Shortcuts](zeron://settings/shortcuts), under **Needs you and running**.

## Going back and forward

Zeron keeps a history of where you have been: chats, drafts, Settings pages, Inbox, Tasks, a task's page, Supervisor, Usage, pull requests, and these docs. To walk it:

- Click the **Back** and **Forward** arrows in the titlebar.
- Press the back and forward buttons on your mouse.
- Swipe left or right with two fingers on the trackpad.

While you swipe, a round pill slides in from the edge of the window and fills as you go. Once it is far enough it changes color and names the page you will land on; let go to switch. Let go early and the page springs back. With nowhere to go, the pill reads **No previous page** or **No next page**.

On a trackpad that supports it, you feel a light tick when the swipe is far enough to switch.

> Swiping follows the macOS trackpad setting **Swipe between pages**. If swiping does nothing, check that it is turned on in System Settings.

## Session tabs

The titlebar shows a tab for each session open on this device. A tab shows the agent's mark (which changes while the agent works), the title, and a status dot that turns into a close button when you hover the tab or select it. The tabs of a Task's lead and its steps gather into one group behind a tab with the Task's title; see [Agent trees and plans](zeron://docs/mission-control).

![Session tabs in the titlebar, with a Task's tab group and the tab menu open](getting-around-tabs.jpg)

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

The open chat can show a right pane with Changes, Files and a browser as tool tabs, and its agents, forks and side chats as tabs beside them. Toggle it with Cmd+R (**Toggle right sidebar**). Terminals live in the dock under the chat, which Cmd+J opens and closes. See [Right pane](zeron://docs/right-pane).
