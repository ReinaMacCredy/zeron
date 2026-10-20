A sidechat lets you ask a quick question about a chat without interrupting it. The main agent keeps working while a separate agent answers you on the side.

## Open a sidechat

- Type `/btw` followed by your question in the composer and send it. Zeron opens a new sidechat and sends the question as its first message. `/sidechat` does the same.
- Send `/btw` on its own to reopen the sidechat you used last.
- Click the tab on the top edge of the composer. It reads **New sidechat (btw)**, or **Sidechats (btw)** with a count once you have some. A dot on it shows when the latest sidechat is working, waiting for you, or failed.
- Press Cmd+; to open or close sidechats. Rebind it as **Toggle sidechat (btw)** in [Settings > Shortcuts](zeron://settings/shortcuts).

You need an existing chat first; the new chat canvas has no sidechats.

## Working in the sidechat panel

The sidechat takes the place of the main composer, marked with a **btw** pill. Each sidechat gets a tab named after its first question.

![Sidechats above the composer](side-chat-tabs.jpg)

- **+** (**New btw tab**) opens another sidechat.
- The × on a tab (**Archive sidechat**) archives it.
- A **Draft** chip shows the unsent text you left in the main composer, so nothing is lost.
- **Open in side pane** moves the current sidechat to the right pane, where it gets a **Sidechat** tab.
- Press Escape, or click the **Esc** chip, to close the panel and return to the main composer.

You can also start one from the right pane's + menu with **New sidechat**. The [Live state](zeron://docs/live-state) column lists your sidechats under **Side chats**; click one to open it in the right pane.

## What the sidechat knows

A sidechat runs in the same project and branch with the same agent and model as its chat, but in its own session. It receives a copy of the chat's conversation as reference. It is told to answer your question rather than continue the chat's task, and not to change files or git unless you ask it to.

Images or appshots you attached in the main composer move into the sidechat when you send `/btw` with a question.

Sidechats drop off the tab after 24 hours and show as **Expired** in the **Live state** column.

## Sidechats, side chats, and forks

Three similar names do different things:

| Name | Where you start it | What you get |
| --- | --- | --- |
| Sidechat | `/btw`, the tab on the composer, or **New sidechat** in the right pane's **+** menu or the **Live state** column | A separate session that answers you about this chat, with a copy of its conversation as reference. In the right pane its tab reads **Sidechat**. |
| **New side chat** | Hover the **Chats** header in the Files panel, or its empty state | An empty chat beside this one, with the same project, agent, and model but none of its history. Nothing is saved until you send its first message. |
| **Fork chat** | **Side chats** in the **Live state** column, or **Fork this chat** (**Fork** when empty) in the Files panel's **Chats** section | A copy of this conversation through its latest completed reply, as a new chat that carries on by itself. |

Side chats and forks open in the right pane with their own composer, and the Files panel lists them under **Chats**. See [Right pane](zeron://docs/right-pane).
