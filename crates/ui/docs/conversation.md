The open chat shows your prompts on the right and the agent's replies, tool calls, and status rows below each other. This page covers reading that transcript and acting on it. To write and send, see [The composer](zeron://docs/composer).

## Messages

Hover a message to see the strip under it:

| Item | What it does |
| --- | --- |
| Time | When the message was sent, such as **Oct 20, 3:45 PM**. Replies show it once the turn has finished. |
| **Copy message** | Copies the message text. For a reply, that is the agent's written text as Markdown, without tool output. The icon turns into a check for a moment. |
| Branch icon (replies) | Forks the chat from this reply. See Fork or rewind below. |
| Restart icon (your prompts) | Rewinds the chat to before this prompt. |

![Hovering a reply shows its time, Copy message, and fork](conversation-hover.jpg)

A long prompt (more than five lines, or about 400 characters) is clipped. Click **Show more** under it to read the rest and **Show less** to fold it again, or press and hold the bubble.

Images you attached show as thumbnails above your message. Click one to see it full size.

Some rows are not ordinary messages:

- A message another chat sent here reads **From** and the sender's name. See [Agents and other threads](zeron://docs/threads).
- Rows about delegated tasks and subagent chips are covered in [Delegation and subagents](zeron://docs/delegation).
- Compaction and classifier rows are covered in [Compaction and context](zeron://docs/compaction).
- A handoff starts with a divider covered in [Hand off a thread](zeron://docs/handoff).

## Move around

With two or more prompts and a wide enough window, short ticks run down the left edge of the transcript, one per prompt you sent. The tick for the part you are reading is brighter. Hover a tick to preview the prompt and the start of the reply; click it to jump there.

While you are at the end of the chat, it follows new output as it arrives. Scroll up and it stays where you are. Once you are well above the end, a **Scroll to bottom** pill floats above the composer; click it to return.

## Fork or rewind

**Fork from a reply.** Hover a reply and click the branch icon. The **Fork chat from here** dialog shows which reply you picked. Click **Fork in this workspace** to start a new chat with the conversation up to and including that reply; the original chat stays as it is. The new chat opens, and its transcript starts with a **Forked from** marker naming the source chat. Click the marker to go back to the source.

**Rewind to a prompt.** Hover one of your prompts and click the restart icon. The **Rewind to here** dialog offers **Restore conversation**: it removes this prompt and everything after it and puts the prompt back in the composer so you can edit and resend it. Files on disk stay as they are; only the conversation is rewound.

When the icon is dimmed, the action is not available, and clicking it shows the reason in the composer:

- **Cursor does not support fork or rewind** (or the name of your agent): this agent cannot branch a conversation.
- **Stop the running turn before rewinding**: rewind waits until the agent is idle. Forking works while it runs.

Prompts that have not been sent yet and subagent transcripts have no fork or rewind.

Two other ways to branch a chat:

- **Fork chat** in the **Live state** column copies the conversation through the latest finished reply into a new chat in the right pane. See [The Live state column](zeron://docs/live-state).
- A sidechat answers a quick question on the side without changing this chat. See [Side questions with /btw](zeron://docs/side-chat). A sidechat's transcript starts with a **Sidechat of** marker.

## Links

| You do | What happens |
| --- | --- |
| Click a link to a file in the project | The file opens in the right pane. |
| Click a web link | It opens in a Browser tab in the right pane, or in your default browser if **Open links in Zeron** is off. |
| Cmd+click a web link | It opens in your default browser. |
| Hover a link | Its destination shows. File links show their path, relative to the project when the file is inside it. |
| Right-click a link, or press Shift+F10 on a focused link | The link menu opens. |

Clicking a link while text is selected does nothing, so you can select across links. Press Tab to move focus between links.

The link menu for a web link has **Open in Zeron**, **Open in external browser**, **Copy link address**, and a checkable **Open links in Zeron**. That last row sets what a plain click does, for every chat; it is on by default, and this menu is the only place to change it.

The menu for a file link has **Open in Zeron**, **Open with default app**, **Show in folder** (files on this device), and **Copy file path**, which copies the full path.

When an agent names a file in inline code, such as `SOURCES.md`, and that file exists, the name becomes a link to it.

## Code and diagrams

Code blocks show their language in the header and have two buttons:

- **Copy code** copies the block. It reads **Copied** for a moment.
- The wrap button switches between **Fit content** (long lines wrap) and **Use horizontal scrolling**. The choice applies to every code block and is remembered.

A `mermaid` code block draws as a diagram once the agent has finished writing it. **Show source** swaps the diagram for its text and **Show diagram** swaps it back. Click the diagram to enlarge it. If a diagram cannot be drawn, the source stays with a warning icon; hover it to see why, such as **Diagram could not be rendered**.

Wide tables scroll sideways. Math is covered in [Appearance](zeron://docs/appearance).

## Images

Click an image in the transcript (an attachment, an image the agent made, or a picture in a tool result) to open it full size:

- Pinch, or Ctrl+scroll, to zoom. Scroll or drag to pan.
- Press Escape or click to close.

Right-click an image for **Copy path** and **Copy image**.

## Quote a reply

Drag across the transcript to select text; selections can span messages. Double-click selects a word and triple-click a block. Cmd+C copies the selection (Ctrl+C on Windows and Linux).

When you let go, an **Add to chat** pill appears under the selection. Click it to stage the text as a quote. The composer shows a **1 selection** chip (or **N selections**); hover it to see what you picked, and click the × on it to drop all of them. When you send, the quotes go after your message as quoted text. Sending quotes with no text of your own asks the agent to refer to them.

**Add to chat** works in sidechats and split panes, but not in a read-only subagent transcript. To quote from the terminal, see [Terminal](zeron://docs/terminal).

## The agent's work

The agent's tool calls fold into groups. Each group's title sums up what happened, such as **Ran 3 commands · edited 2 files**, **Read 1 file · searched 2 times**, **Thought 2 times**, or **Used Safari · 1 action**. A group stays open while the agent is still working in it, then folds to its title. Click the title to open or close it. Subagent chips and **Started thread** rows stay outside the groups.

![The agent's tool calls grouped under one summary](conversation-tools.jpg)

Inside a group, each chip names one call. Click a chip to see exactly what ran and its output or diff.

| On a chip | What it does |
| --- | --- |
| A file name | Opens the file in the right pane: a **Working tree** Diffs tab scrolled to it when the file has uncommitted changes, otherwise the file itself. |
| A violet skill name | Opens that skill's `SKILL.md`. See [Commands and mentions](zeron://docs/commands-and-mentions). |
| **Show full output** / **Show full diff** | Long output is cut short; this loads the rest. The label may include the size, such as **Show full output (12 KB)**. |

The agent's reasoning shows as a **Thought process** chip; click it to read it.

When the agent asked you a question, a **Question** row stays in the transcript with the question's header, or **Awaiting your answer…** until you answer. Answering is covered in [When an agent asks](zeron://docs/agent-questions).

## Compact mode

Turn on **Compact mode** ("Collapse thinking and tools.") in [Settings > General](zeron://settings/general) to see less of the work. Each turn's tool calls, thinking, subagent chips, and in-between text fold under one header, and only the final reply stays in view. Text the agent wrote along the way becomes **Wrote** chips. Nothing opens by itself; click the header to look inside.

While the turn runs, the header sums up the work. When the turn finishes, it changes to **Worked for** and the time, such as **Worked for 1m 35s**.

## While the agent works

Under the reply that is streaming, a working row shows a spinner, a word that changes every few seconds (**Zeroning…**, **Thinking…**, **Pondering…**, and others), how long the turn has run, and the speed in tokens per second (**· 42 tok/s**).

The row can also read:

| Label | Meaning |
| --- | --- |
| **Sending…** | Your message is on its way and the turn has not started yet. |
| **Consulting advisor…** | The agent is waiting on an advisor's answer. |
| **Queued — will send automatically** | You are offline. Your message is saved and goes out when the connection is back. |
| **Not delivered — click to retry** | The message did not reach the agent. Click to send it again. |

The agent can run shell commands in the background. While any run, a **N Running** pill shows near the composer. Click it to list them under **Background shells**; open one to read its latest output, **Copy** it, or stop it. The separate pill for subagents is covered in [Delegation and subagents](zeron://docs/delegation).

To stop the agent, see [The composer](zeron://docs/composer).

## When a turn stops

If a run fails, **Run failed** shows above the composer and the error appears in the transcript as an **Error** tile with the message.

When the last turn was stopped or failed and nothing is running, the send button on an empty composer becomes a play button (**Resume**). Click it to have the agent carry on without asking. The transcript marks this with a **Resumed** divider.

## The todo list

When the agent keeps a checklist, it docks above the composer. The header reads **Todo** and how many items are done, such as 2/5, followed by the current item, or **All done**. Click the header to show or hide the full list. Long lists fold around the current item; click **N earlier** or **N later** to see the rest.

While the agent is idle, the × (**Dismiss**) hides the list. It comes back when the agent writes a different one.

## Pages and visuals

An agent can publish an HTML page into the chat. See "Pages rendered in the chat" in [Browser](zeron://docs/browser).

Agents can also show native visuals in a reply: stats, tables, charts, drawings, and 3D scenes. These are interactive (buttons, tabs, toggles, and sliders change what they show) and, unlike HTML pages, they render on every platform.

The column to the right of the chat is covered in [The Live state column](zeron://docs/live-state).
