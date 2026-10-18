A sidechat lets you ask a quick question about a chat without interrupting it. The main agent keeps working while a separate agent answers you on the side.

## Open a sidechat

- Type `/btw` followed by your question in the composer and send it. Zeron opens a new sidechat and sends the question as its first message. `/sidechat` does the same.
- Send `/btw` on its own to reopen the sidechat you used last.
- Click the tab on the top edge of the composer. It reads **New sidechat (btw)**, or **Sidechats (btw)** with a count once you have some. A dot on it shows when the latest sidechat is working, waiting for you, or failed.
- Press Cmd+; to open or close sidechats. Rebind it as **Toggle sidechat (btw)** in [Settings > Shortcuts](zeron://settings/shortcuts).

You need an existing chat first; the new chat canvas has no sidechats.

## Working in the sidechat panel

The sidechat takes the place of the main composer, marked with a **btw** pill. Each sidechat gets a tab named after its first question.

- **+** (**New btw tab**) opens another sidechat.
- The × on a tab (**Archive sidechat**) archives it.
- A **Draft** chip shows the unsent text you left in the main composer, so nothing is lost.
- **Open in side pane** moves the current sidechat to the right pane, where it gets a **Sidechat** tab.
- Press Escape, or click the **Esc** chip, to close the panel and return to the main composer.

You can also start one from the right pane's + menu with **New sidechat**. The **Live state** column lists your sidechats under **Side chats**.

## What the sidechat knows

A sidechat runs in the same project and branch with the same agent and model as its chat, but in its own session. It receives a copy of the chat's conversation as reference. It is told to answer your question rather than continue the chat's task, and not to change files or git unless you ask it to.

Images or appshots you attached in the main composer move into the sidechat when you send `/btw` with a question.

Sidechats drop off the tab after 24 hours and show as **Expired** in the **Live state** column.

> **Fork chat** in the **Live state** column is different: it copies the conversation into a new chat in the right pane that carries on by itself.
