Zeron has a built-in browser in the right pane, so your local app or a website sits beside the conversation. Agents can use it too, and HTML pages they write open in it or render in the chat.

## Open a browser tab

Click **Browser** in an empty right pane, or click **+** and choose **Browser**. In a chat, a new tab lists the dev servers running in this project under **Running locally** (or **Running on your device** for a chat on another device); click **Open** next to one. A server appears when a program started in the project folder listens on a port and answers web requests. This works for chats that run on a Mac or a Linux machine.

Or choose **Or enter a website address** and type an address such as `example.com` or `localhost:3000`. The field takes http and https addresses only; it does not search the web. A tab that is not tied to a chat, such as one opened on New chat, shows **Preview your work** with an **Enter an address** button instead of the server list.

The toolbar has **Back**, **Forward**, **Reload page**, the address field, and **Open in default browser**. While the browser has focus:

| Shortcut | Action |
| --- | --- |
| Cmd+L | Focus the address field |
| Cmd+T / Cmd+W | New tab / close tab |
| Cmd+[ / Cmd+] | Back / forward |
| Cmd+Shift+R | Reload (**Reload browser page** in [Settings > Shortcuts](zeron://settings/shortcuts)) |

## What is kept after a restart

Browser tabs close when Zeron quits, like every right-pane tab. What a site stores, such as your logins and cookies, is kept for next time on macOS 14 or later and on Linux. On macOS 12 and 13 it is kept only until Zeron quits, and a notice under the toolbar says so. On Windows the browser is private and keeps nothing.

That data belongs to a browser profile: one per device for a local workspace, and one per signed-in account and workspace for a synced workspace (see [Devices](zeron://docs/devices)). Turning sync on or off, or signing in or out, switches the profile and closes your open browser tabs.

## Agents in the browser

On macOS, an agent can open and drive a tab for its own chat. It can open a new tab only on a localhost address, and only while that chat is open on the computer that runs it. In a tab it already controls, it can go to any http or https address. **Developer access** in [Settings > Computer](zeron://settings/computer) also lets tasks run JavaScript in their tabs.

When you stop the agent's computer use, the agent loses control of the tab. Only you can hand it back, with **Resume agent** in the toolbar on macOS. Annotating a tab also pauses the agent, but only while you annotate: it gets the tab back when you finish or cancel, unless it was already paused.

## Annotate

On macOS, click the pencil (**Annotate elements, regions, and drawings**) on a loaded page to mark it up. A toolbar at the top of the page has **Select** (V) to pick elements, **Region** (R) to draw a box, **Draw** (D) for freehand marks, and **Erase** (E) to remove one. The expand button at the left end of the row that holds **Attach** opens a style panel where you can try other font, color, border, and spacing values on what you picked.

![Annotating a page in the browser](browser-annotate.jpg)

Click **Attach** or press Enter. Zeron attaches a screenshot of the marked area to the composer as an [Appshot](zeron://docs/appshots), carrying the page title, URL, and details of the picked elements, adds any requested style changes to your draft, and moves focus to the composer, where you describe the change. You type it there rather than on the page so the page's own scripts cannot change your words. Nothing is sent until you send it. Press Escape, or click the pencil again, to cancel.

## HTML files the agent mentions

On macOS, when an agent's message links or names an `.html` file, it shows a card with the page title and path. **Open in** opens it in your default browser, or in the app you picked last. The arrow next to it lists **Zeron** (a browser tab in the right pane), your browsers, **Reveal in Finder**, and **Copy path**.

## Pages rendered in the chat

An agent can also publish an HTML page straight into the chat, sized to fit. Click the expand arrow on its header to open it larger, switch between **Live** and **Source**, or click **Save .html** to keep a copy. Press Escape to close it. Live pages show on macOS; elsewhere you read the source.

![An HTML page in the chat](browser-html-card.jpg)
