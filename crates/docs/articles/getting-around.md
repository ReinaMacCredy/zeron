The Zeron window has three parts: the app rail on the far left, the sidebar with your sessions, and the main column where the open chat or page lives.

![The Zeron window: app rail, sidebar, and an open chat](getting-around-window.jpg)

## The app rail

The narrow strip on the left switches between Zeron's pages. Hover a button to see its name.

| Button | What it opens |
| --- | --- |
| **New chat** | [New chat](zeron://docs/new-chat), with a composer and the **Needs you**, **Running** and **Recent** cards. Cmd+N opens it from anywhere. |
| **Inbox** | The [Inbox](zeron://inbox): the history of what your agents asked, finished and failed today. |
| **Tasks** | The [Tasks](zeron://tasks) page. A number counts the tasks you captured and have not seen yet. |
| **Skills** | The [Skills](zeron://docs/skills) page: every skill your agents find. |
| **Pull requests** | The [Pull requests](zeron://pull-requests) board. |

Between those buttons and the ones at the foot, the rail shows a mark for each chat you have open as a tab: the agent's mark, with a dot under it while the chat works, asks or has failed, or a short running track while it waits on background work. The chat you are on stands on a tile the color of the chat card. Hover a mark for the chat's title, click it to go there, and right-click it for the tab's menu. The marks of pinned tabs stand first, above a short rule; a split shows its chats' marks joined in one cell. New chat tabs have no mark. The arrow above the marks folds them: they slide up into a small deck under it, with how many there are and a dot for the most urgent one (asking, then failed, then working). Click the deck or the arrow to unfold them. Zeron remembers whether they are hidden.

What needs you right now is not on the rail: questions, plans and decisions wait in the **Waiting** pill above the composer, and the pill on the right of the titlebar counts the rest, with Cmd+I for its tray. See **Needs you and running** below.

Further down the rail:

- The **usage ring** shows the tightest limit window of the agent accounts you use. Hover it for a card with each account's windows and when they reset, and click it to open the [Usage](zeron://usage) page. At a limit it turns red. See [Usage](zeron://docs/usage).
- A microphone button for voice mode, when Codex is installed and turned on in [Settings > Providers](zeron://settings/providers). See [Voice and dictation](zeron://docs/voice).
- **Help**: these docs, what's new, keyboard shortcuts, **Star Zeron on GitHub**, and **Agent updates** when an agent CLI has one. See [Updates and what's new](zeron://docs/updates).
- A download button, when an app update is available.
- **Settings** at the bottom. Cmd+, opens and closes it too. Settings opens as a panel over the app, with a tree on the left (General, Appearance, Notifications, Providers, Agents, Context & tools, This Mac, Devices & updates, then **Account** and **Lab**) and **Search settings** on top. A group with more than one page opens to them under a guide line while you are on one of them. **Lab** holds what is still being tried: compaction experiments, agent access and Zeron Beta. Press / or Cmd+F to search: the tree lists each found setting with its path, such as **Appearance › Advanced look**, and picking one opens its page, glides to that row and flashes it. The header names the section, for example **Settings › Devices & updates**, and holds that page's actions. **Account** shows who you are signed in as, or **Stored on this device**, with **Enable sync** or **Sign out**. To sync with your other devices, see [Devices](zeron://docs/devices). Close it with the **×**, Escape, or a click outside the panel.

## The sidebar

The sidebar lists your sessions, drafts, and projects. Its header has **Search** (which opens the [command palette](zeron://docs/command-palette)), a project filter, **New project**, and **New session**. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

Drag the sidebar's right edge to resize it, from 242 to 400 px wide. Double-click the edge to return to the default width.

To hide or show the sidebar, click the sidebar button at the top left of the window or press Cmd+B (**Toggle left sidebar**). Zeron remembers your choice.

With the sidebar hidden, move the pointer just right of the app rail and it slides out as a floating panel. Move away and it hides again; press Escape to dismiss it. Turn this off with **Reveal sidebar on hover** in [Settings > Appearance](zeron://settings/appearance).

## Needs you and running

Questions from other chats and plans waiting for your OK wait in the **Waiting** pill above the composer, which reads **1 question waiting** or the like, where you answer them without leaving the chat you are in. See [When an agent asks](zeron://docs/agent-questions).

The pill on the right of the titlebar counts the rest: threads that failed, Tasks whose message did not send or that are ready to review, and the ones running, for example **1 needs you · 2 working**. With nothing failed or waiting it reads only the running count, and with nothing at all it is hidden.

Click it or press Cmd+I to open its tray:

- The header says what needs you and what is running, once.
- **Needs you** lists each thread that failed and each Task that waits on you, and shows only when there is one. **Open** opens it.
- **Running** lists each thread that works, with its agent's mark, its time on the right and its subagents under it.
- **History** opens the Inbox and **Tasks** opens Tasks.

Escape, a click outside or Cmd+I closes the tray. Change the shortcut in [Settings > General > Shortcuts](zeron://settings/shortcuts), under **Needs you and running**.

## Going back and forward

Zeron keeps a history of where you have been: chats, drafts, Settings pages, Inbox, Tasks, Usage, pull requests, and these docs. To walk it:

- Click the **Back** and **Forward** arrows in the titlebar.
- Press the back and forward buttons on your mouse.
- Swipe left or right with two fingers on the trackpad.

While you swipe, a round pill slides in from the edge of the window and fills as you go. Once it is far enough it changes color and names the page you will land on; let go to switch. Let go early and the page springs back. With nowhere to go, the pill reads **No previous page** or **No next page**.

On a trackpad that supports it, you feel a light tick when the swipe is far enough to switch.

> Swiping follows the macOS trackpad setting **Swipe between pages**. If swiping does nothing, check that it is turned on in System Settings.

## Session tabs

The top of the chat card has a notch, as a browser has: the card rises to the window's top at both corners, and between them the window dips in and holds a tab for each session open on this device. **Back**, **Forward** and **New tab** sit in the left corner, and the chat's controls in the right one. With the sidebar hidden the tabs start beside the window's buttons, and with the right pane open they run to the card's edge.

A tab shows the agent's mark (which changes while the agent works), the title, and a status dot that turns into a close button when you hover the tab or select it. While a chat waits on work its agent left running in the background, the dot is a prompt arrow and a short track with a pill running along it. The tab you are on is part of the chat card: it rises from the card through the notch's floor, as a browser's tab joins its page, and slides to the next tab you pick.

![Session tabs in the notch at the top of the chat card, two of them pinned, and their marks on the rail](getting-around-tabs.jpg)

Selecting a session anywhere (the sidebar, the command palette, a notification) opens its tab. A chat you started opens its tab again by itself when it begins a new run, and any chat that starts waiting for your input opens its tab too.

Closing a tab only hides it here. The session keeps running and stays in the sidebar, where a click opens it again.

- Click a tab to switch to it.
- Middle-click a tab, or click its close button, to close it.
- Drag a tab to reorder it, or drag it onto the chat to open a split. See [Split view](zeron://docs/split-view).
- Right-click a tab for **Pin tab**, **Close tab**, **Close others**, **Close to the right**, **Close to the left**, and **Close idle tabs**. **Close idle tabs** closes idle sessions except the one you are viewing, the sessions in a split, and drafts.

### Pinned tabs

**Pin tab**, in a tab's menu or as the pin button that shows between the tick and the close button when you hover a tab, turns it into a small tile with only the agent's mark and its status dot, at the head of the tabs before a short rule. Hover the tile for the chat's title. Pinned tabs stand in the order you pinned them, and dragging reorders them among themselves.

A pinned tab stays until you unpin it: it has no close button, Cmd+W and a middle-click leave it, and **Close others**, **Close to the right**, **Close to the left** and **Close idle tabs** skip it. Hover the tile and click the unpin button at its corner, or right-click it and choose **Unpin tab**, to make it a normal tab again, first among the unpinned ones.

Only a tab of one chat can be pinned, and a chat dropped on a pinned tab does not join it. A pinned tab that becomes a split is unpinned. Pinning a tab is separate from **Pin chat** in the sidebar.

Cmd+W closes the active tab in the right pane first, then the session tab you are on. With neither left, it closes the window. Click Zeron's Dock icon to open the window again.

## These docs

Choose **Docs** in the rail's **Help** menu to open these pages. **Overview** shows every article as a card, grouped by section. The list on the left has a search field that matches article titles and text.

The buttons under an article's title open the page or setting it describes. **Previous** and **Next** at the bottom step through the articles in order.

## The right pane

The open chat can show a right pane with Changes, Files and a browser as tool tabs, and its agents, forks and side chats as tabs beside them. Toggle it with Cmd+R (**Toggle right sidebar**). Terminals live in the dock under the chat, which Cmd+J opens and closes. See [Right pane](zeron://docs/right-pane).
