A side chat lets you ask a quick question about a chat without interrupting it. The main agent keeps working while a separate agent answers you on the side, in a tab of the right pane.

## Open a side chat

- Type `/btw` followed by your question in the composer and send it. Zeron opens a new side chat in the right pane and sends the question as its first message. `/sidechat` still works when you type it, but the `/` list shows only `/btw`.
- Click the **Side chat** tab on the top edge of the composer, press Cmd+;, or send `/btw` on its own. Each brings back the side chat already open in the pane, else your newest live one, else starts a new one. A dot on the tab shows when a side chat is working. Rebind the shortcut as **Side chat** in [Settings > General > Shortcuts](zeron://settings/shortcuts).

You need an existing chat first; New chat has no side chats.

![A side chat open in the right pane, beside its chat](side-chat-pane.jpg)

## Working in a side chat

A side chat opens as a tab in the right pane with a side chat badge, named after its first question (**New side chat** until you send one). It has its own composer, so the main composer keeps your draft. Its header says which chat it belongs to and that it got that chat's conversation as reference.

The Sidecar lists your side chats under **Beside this thread**; click one to open its tab again. See [Live state](zeron://docs/live-state).

## What the side chat knows

A side chat runs in the same project and branch with the same agent and model as its chat, but in its own session. It receives a copy of the chat's conversation as reference. It is told to answer your question rather than continue the chat's task, and not to change files or git unless you ask it to.

Images or appshots you attached in the main composer move into the side chat when you send `/btw` with a question.

Side chats expire after 24 hours and show as **Expired** in the Sidecar.

## Side chats, new side chats, and forks

Three similar things open beside a chat, each as a tab in the right pane:

| Name | Where you start it | What you get |
| --- | --- | --- |
| Side chat | `/btw`, the **Side chat** tab on the composer, or Cmd+; | A separate session that answers you about this chat, with a copy of its conversation as reference. |
| **New side chat** | Hover the **Chats** header in the Files tab, or its empty state | An empty chat beside this one, with the same project, agent, and model but none of its history. Nothing is saved until you send its first message. |
| Fork | Hover a reply and click **Fork chat from here**, or **Fork this chat** (**Fork** when empty) in the Files tab's **Chats** section | A copy of this conversation through that reply (or the latest completed one), as a new chat that carries on by itself. |

All three get a badge for their kind on their tab, and the Sidecar and the Files tab's **Chats** section list them. See [Right pane](zeron://docs/right-pane).
