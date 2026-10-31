The composer is the message box at the bottom of every chat. You type your prompt there, attach files, and decide what happens when you send while the agent is still working.

Its bar carries, from left to right, the paperclip, the **Plan** switch, the access chip (**Supervised**, **Auto-accept edits** or **Full access**), the model chip with the model and effort, and the send button. The access chip is always there, in the same spot on New chat and in a chat; below a narrow width it shows as an icon. See [When an agent asks](zeron://docs/agent-questions) and [Models and effort](zeron://docs/models-and-effort). The **Side chat** tab on the composer's top edge opens a [side chat](zeron://docs/side-chat).

## Sending

Press Enter to send and Shift+Enter for a new line. To send with Cmd+Enter instead, change **Send messages with** in [Settings > General > Chat](zeron://settings/chat).

Press Up on the first line to bring back your earlier prompts in this chat, and Down to return to your draft.

Every composer keeps its own draft: each chat, each side chat, and New chat. Text, attachments, and Appshots you have not sent survive a restart. In the sidebar, a chat with unsent text shows an amber pen.

A sent message that uses Markdown, such as a list, a heading, or a code block, shows rendered in the transcript, and your line breaks stay as typed. Plain text and messages with chips show as you typed them.

## Attachments and images

- Click the paperclip (**Attach images**) to pick files. Any file works, not only images.
- Drag files onto the conversation (**Drop to attach**).
- Paste an image or a copied file.

Each attachment appears as a chip in your text: images are named **Image 1**, **Image 2**, and so on. Click an image chip to see it full size, and press Escape to close it. To remove an attachment, delete its chip. Files can be up to 24 MB each.

An attachment that has no chip in your text shows as a small tile above it instead. Hover the tile and click its **×** to remove it.

## Pasted text

A paste of 5,000 characters or more turns into a **Pasted text** chip that shows the paste's first line, so a long log does not bury your prompt. Click the chip (**Click to show in text field**) to expand it back into plain text. Left as a chip, the paste goes to the agent as an attached text file named after its chip, such as **Pasted text 1.txt**. After you send, the message shows a **Pasted text** pill with its line count; click it to read the full paste in a read-only preview, and press Escape to close it.

## Comments and quotes

Comments you leave on a diff or a file, and [review](zeron://docs/review) findings you hand to the agent, wait above your text as a **1 comment** or **N comments** chip. Click the chip for the list: each row shows the file and line, with a button to remove that one, and **Clear all** removes them all. They go out with your next message, and you can send comments with no text at all.

Text you quote from the transcript or the terminal shows as a **1 selection** or **N selections** chip. Hover it to read the quoted text, or hover and click its **×** to drop every quote. How to pick text to quote is in [Reading a conversation](zeron://docs/conversation) and [Terminal](zeron://docs/terminal).

## When the agent is still working

While a run is going, the send button's tooltip reads **Queue message**. What a follow-up does depends on **Follow-up behavior** in Settings > General > Chat:

- **Queue** (the default) holds your message and sends it when the current turn ends.
- **Steer** delivers it into the running turn right away. Some agents only take input between turns; with one of those, a caption under the composer says **This agent can't be steered mid-turn. Your message will be queued and sent when the current turn finishes.**

![Queued follow-ups above the composer](composer-queue.jpg)

With Enter as the send key, Cmd+Enter does the opposite of the setting for one message.

Queued messages wait in a tray above the composer. Drag a row to reorder, click **Edit** to change it in the composer (**Save to queue** or **Cancel**, or press Escape), or **Remove** it. Each row also has a button to deliver it early, and which one you get depends on the row and the agent:

- **Send now** when the row has attachments. It interrupts the run.
- **Steer** when the agent takes input mid-turn. The current work keeps running.
- **Send next** otherwise. It goes in at the next turn.

With an empty composer, Cmd+Enter does the same as clicking the newest row's button. Hold Cmd to see the hint (**⌘ Enter**, or **⌘↵** in a narrow window) on that button.

## Stopping the agent

With an empty composer during a run, the send button becomes **Stop**. You can also type `/stop`. To stop with Escape, turn on **Stop active agent with Escape** in Settings > General > Chat; it is off by default. Enter never stops a run. Stop ends the agent's current turn only: subagents and background shells it started keep running and report back when they finish. Stop those from the Sidecar's **Beside this thread** list, one by one or with **Stop all** (see [Delegation and subagents](zeron://docs/delegation)). On Pi, and when a stop does not take effect within 15 seconds, Stop ends the agent's process instead, and its subagents and background shells end with it. A task the agent delegated to another chat during the stopped turn keeps running either way, but it does not wake the agent when it finishes: its chip and its row in the Sidecar show the result.

When a turn stopped short (you stopped it, or it ended with an error) and the composer is empty, the button becomes a play triangle (**Resume**). Click it to have the agent pick up where it left off.

## Predictions

Turn on **Show predictions** in Settings > General and Zeron suggests your next message after each reply. The suggestion appears in the empty composer; press Tab to accept it. Under **Predict with**, choose **Agent** or **Custom API**.

- **Agent**: the **Agent model** row reads **Session agent** by default. A Claude Code or Codex chat then predicts with its own agent on that agent's cheapest model; any other chat uses the first enabled of Claude Code or Codex, and gets no predictions if neither is enabled. Click the chip to pick one model for every chat instead, and **Reset** to go back to **Session agent**.
- **Custom API**: any OpenAI-compatible API. Fill in **Base URL** (such as `https://api.openai.com/v1`) and **Model**, then click **Save**. The URL must use https; plain http works only for localhost. Copy your API key and click **Paste key** to store it in the Keychain, or **Remove key** to delete it. Local servers like Ollama need no key.

## Plan first

Turn on the **Plan** switch in the composer bar, or press Shift+Tab, to have the agent write a plan before it changes anything. On a list line, Shift+Tab outdents the line instead. While Plan is on, the composer's border takes the accent color and Claude Code's send button reads **Start plan**. The pull request chat does not have the switch.

When Claude Code's plan is ready, it arrives in the transcript as a card with its steps and three buttons:

- **Approve** runs the plan in this chat. The card then reads **Running here**.
- **Approve as Task** tracks the plan's steps in [Tasks](zeron://docs/mission-control): the card turns into the Task with the step it is on and **Open in Tasks ›**, and each step reports back to this chat. It is dimmed, with the reason on hover, when this chat cannot become a Task, for example when it already leads one.
- **Edit plan** tells the agent no, keeps plan mode on, and puts the cursor in the composer so you can say what to change.

![A plan card waiting for your approval](plan-card.jpg)

The Cmd+I tray and the **Needs you** card on New chat offer the same **Approve** and **Approve as Task**. On Codex and OpenCode, Plan has the agent write a plan and change nothing, with no approval step, and the send button keeps its arrow.

## Notices above the composer

- When the chat's device cannot be reached, a quiet caption says **Offline — messages will send when you're back online.** or **Messages will send once the connection recovers.** You can keep sending; the messages go out once the connection is back.
- When a send or a run fails, a chip shows the reason: **Warning** in amber when the engine is not connected, **Error** in red otherwise. Click it to dismiss it.
- **Compacting context…** shows while the chat is being [compacted](zeron://docs/compaction).
- When the chat's agent reaches a usage limit, one notice says which limit and when it resets, with a button to switch to the model Zeron suggests (or **Switch model** to pick one) and **Wait for reset**. While Zeron waits for the reset, the notice says so and offers **Cancel**. See [Usage and cost](zeron://docs/usage).

## The session footer

In an open chat, a row under the composer carries, from left to right:

- For a git project, where the chat works: **Local checkout** or **Worktree**, then the branch (**No ref** when it is not known).
- For a git project, the session stats chip (see [Usage and cost](zeron://docs/usage)) and a pull request badge when the branch has one.
- The context ring, which shows how full the context window is. See [Compaction and context](zeron://docs/compaction).
- The terminal button, which opens the dock under the chat, like Cmd+J. See [Terminal](zeron://docs/terminal).

![The session footer under the composer](composer-footer.jpg)

The checkout and branch labels are read-only: you pick them on New chat.

> To use `/` commands, `$` skills, and `@` mentions, see [Commands and mentions](zeron://docs/commands-and-mentions).
