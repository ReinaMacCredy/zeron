The composer is the message box at the bottom of every chat. You type your prompt there, attach files, and decide what happens when you send while the agent is still working.

## Sending

Press Enter to send and Shift+Enter for a new line. To send with Cmd+Enter instead, change **Send messages with** in [Settings > General](zeron://settings/general).

Press Up on the first line to bring back your earlier prompts in this chat, and Down to return to your draft.

A sent message that uses Markdown, such as a list, a heading, or a code block, shows rendered in the transcript, and your line breaks stay as typed. Plain text and messages with chips show as you typed them.

## Attachments and images

- Click the paperclip (**Attach images**) to pick files. Any file works, not only images.
- Drag files onto the conversation (**Drop to attach**).
- Paste an image or a copied file.

Each attachment appears as a chip in your text: images are named **Image 1**, **Image 2**, and so on. Click an image chip to see it full size. To remove an attachment, delete its chip. Files can be up to 24 MB each.

## Pasted text

A paste of 5,000 characters or more turns into a **Pasted text** chip that shows the paste's first line, so a long log does not bury your prompt. Click the chip (**Click to show in text field**) to expand it back into plain text. Left as a chip, the paste goes to the agent as an attached text file named after its chip, such as **Pasted text 1.txt**. After you send, the message shows a **Pasted text** pill with its line count; click it to read the full paste in a read-only preview, and press Escape to close it.

## When the agent is still working

While a run is going, the send button's tooltip reads **Queue message**. What a follow-up does depends on **Follow-up behavior** in Settings > General:

- **Queue** (the default) holds your message and sends it when the current turn ends.
- **Steer** delivers it into the running turn right away.

With Enter as the send key, Cmd+Enter does the opposite of the setting for one message.

Queued messages wait in a tray above the composer. Drag a row to reorder, click **Edit** to change it in the composer (**Save to queue** or **Cancel**), or **Remove** it. Each row also has a button to deliver it early: **Steer**, **Send next**, or **Send now**, which interrupts the run.

## Stopping the agent

With an empty composer during a run, the send button becomes **Stop**. You can also type `/stop`. To stop with Escape, turn on **Stop agent with Escape** in Settings > General; it is off by default.

## Predictions

Turn on **Show predictions** in Settings > General and Zeron suggests your next message after each reply. The suggestion appears in the empty composer; press Tab to accept it. Under **Predict with**, choose **Agent** (each chat's own agent on its cheapest model, or one model you pick) or **Custom API** for any OpenAI-compatible endpoint.

## Mission Control mode

In an open chat, press Shift+Tab, or click **Mission Control** under the composer, to turn your message into a mission: a lead agent plans the work and runs each step on its own thread. The send button reads **Start mission**. Shift+Tab again turns it back into a message. On a list line, Shift+Tab outdents the line instead. Side chats, split panes, and the pull request chat do not have this mode. On the New chat canvas, the **Mission Control** tab does the same: see [Start a new chat](zeron://docs/new-chat). What happens when you start is in [Tasks](zeron://docs/tasks).

> To use `/` commands, `$` skills, and `@` mentions, see [Commands and mentions](zeron://docs/commands-and-mentions).
