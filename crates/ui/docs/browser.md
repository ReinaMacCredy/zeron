Zeron has a built-in browser in the right pane, so your local app or a website sits beside the conversation. Agents can use it too, and HTML pages they write open in it or render in the chat.

## Open a browser tab

Click **Browser** in an empty right pane, or click **+** and choose **Browser**. A new tab lists the dev servers running in this project under **Preview your work**; click **Open** next to one. Or choose **Or enter a website address** and type a URL or `localhost:3000`.

The toolbar has **Back**, **Forward**, **Reload page**, the address field, and **Open in default browser**. While the browser has focus:

| Shortcut | Action |
| --- | --- |
| Cmd+L | Focus the address field |
| Cmd+T / Cmd+W | New tab / close tab |
| Cmd+[ / Cmd+] | Back / forward |
| Cmd+Shift+R | Reload (**Reload browser page** in [Settings > Shortcuts](zeron://settings/shortcuts)) |

## Agents in the browser

An agent can open and drive a tab for its own chat. It can only open localhost addresses, and only while that chat is open on the computer that runs it. **Developer access** in [Settings > Computer](zeron://settings/computer) also lets tasks run JavaScript in their tabs.

When you stop the agent's computer use, the agent loses control of the tab. Only you can hand it back, with **Resume agent** in the toolbar on macOS. Annotating a tab also pauses the agent, but only while you annotate: it gets the tab back when you finish or cancel, unless it was already paused.

## Annotate

On macOS, click the pencil (**Annotate elements, regions, and drawings**) on a loaded page to mark it up. A toolbar at the top of the page has **Select** (V) to pick elements, **Region** (R) to draw a box, **Draw** (D) for freehand marks, and **Erase** (E) to remove one. Type what you want in **Describe the change…**. The expand button next to it opens a style panel where you can try other font, color, border, and spacing values on what you picked.

Click **Attach** or press Enter. Zeron attaches a screenshot of the marked area to the composer as an [Appshot](zeron://docs/appshots), carrying the page title, URL, and details of the picked elements, and adds your comment and any requested style changes to your draft. Nothing is sent until you send it. Press Escape, or click the pencil again, to cancel.

## HTML files the agent mentions

When an agent's message links or names an `.html` file, it shows a card with the page title and path. **Open in** opens it in your default browser, or in the app you picked last. On macOS the arrow next to it lists **Zeron** (a browser tab in the right pane), your browsers, **Reveal in Finder**, and **Copy path**.

## Pages rendered in the chat

An agent can also publish an HTML page straight into the chat, sized to fit. Click the expand arrow on its header to open it larger, switch between **Live** and **Source**, or click **Save .html** to keep a copy. Press Escape to close it. Live pages show on macOS; elsewhere you read the source.
