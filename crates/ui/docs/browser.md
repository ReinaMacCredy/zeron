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

When you stop the agent's computer use, or start Annotate on its tab, the agent loses control of the tab. Only you can hand it back, with **Resume agent** in the toolbar on macOS.

## Annotate

On macOS, click the pencil (**Annotate elements, regions, and drawings**) to mark up the page: pick elements, draw a region, and add a comment. Zeron then attaches a screenshot of the area to the composer and adds your comment, with the page title and URL, to your draft. Press Escape to cancel.

## HTML files the agent mentions

When an agent's message links or names an `.html` file, it shows a card with the page title and path. **Open in** opens it in your default browser, or in the app you picked last. On macOS the arrow next to it lists **Zeron** (a browser tab in the right pane), your browsers, **Reveal in Finder**, and **Copy path**.

## Pages rendered in the chat

An agent can also publish an HTML page straight into the chat, sized to fit. Click the expand arrow on its header to open it larger, switch between **Live** and **Source**, or click **Save .html** to keep a copy. Press Escape to close it. Live pages show on macOS; elsewhere you read the source.
