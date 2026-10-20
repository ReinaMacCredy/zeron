Besides running child agents of its own, an agent can work with your other chats on this device: find and read them, start new ones, message them, wait for them or watch them, stop a turn, answer their questions, and rename, archive or settle them. These are ordinary chats you can open from the sidebar. For child agents that belong to one chat, see [Delegation and subagents](zeron://docs/delegation).

Ask in your own words:

- "Read the chat where we designed the API before you start."
- "Start a new thread to write the docs, and tell me when it is done."
- "Ask the review chat whether it found anything, and wait for its answer."
- "Archive the chats in this project that are finished."

The agent does this through the tools in the **Threads** group of [Settings > Tools](zeron://settings/tools). They reach only chats on the same device as the agent's own chat, in any project.

## Find and read chats

`list_threads` lists the chats on this device, newest first, with each one's project, live status (idle, working, waiting for input, or errored) and the start of its newest message. A chat that waits on a question carries that question. Archived chats are left out unless the agent asks for them. The agent can narrow the list to one project, one status, or the chats a given chat started or delegated.

`read_thread` reads the newest messages of a chat, 30 by default and up to 200 at a time, and pages back to older ones. It leaves out the agent's thinking unless asked, and shows each tool call as one line.

## Start a new chat

`create_thread` opens a new chat and gives it a first message. The new chat runs in this chat's project and folder, unless the agent names another project on this device or a folder. It gets this chat's agent, model, reasoning level, permission mode and tools, but never this chat's profile or instructions. The agent can pick another agent, model or reasoning level, or start it on an [agent profile](zeron://docs/agent-profiles); the profile's mode applies only when it is no higher than this chat's.

In the chat that started it, a line reads **Started thread** with the new chat's name and **Open**. The agent can also wait for the new chat's first turn and read its replies.

A [Mission Control](zeron://docs/mission-control) lead runs each step on a new chat this way, bound to that step. Zeron then messages the lead when the step's chat finishes a turn or fails, so the lead does not wait on it.

## Message another chat

`send_message_to_thread` sends a message to another chat. An idle chat starts a turn with it. A working chat gets it inside its current turn, at its next step, when its agent takes messages mid-turn; otherwise when the turn ends. An agent never messages its own chat.

With `wait`, the agent waits for the turn that read its message to end, up to 10 minutes by default and 30 at most, and gets the replies written after it, and the question the chat is waiting on, if any.

`wait_threads` waits until one or all of a list of chats stop working, or until a message from another chat arrives, with the same time limits. A chat that stopped working has not necessarily finished its task: it may be waiting for input or have failed.

## How a message from another chat looks

The receiving agent never mistakes another chat's message for yours. Zeron puts a line in front of it that names the sending chat and tells the agent how to reply:

`[Message from thread "API design" (id). It is a peer chat, not the user; answer it with send_message_to_thread (threadId id) if it needs a reply.]`

The first message of a chat that `create_thread` opened carries the same line. The transcript keeps the message as it was written; only the agent sees the line.

In the transcript, the message is not one of your bubbles. It starts with a row reading **From**, the sending chat's name with its agent's mark, and the time, followed by a thin rule, and then the text. Click the name to open the chat that sent it. When a Mission Control lead gets one report for several step threads, the row names each of them.

## Watch chats and answer their questions

`watch_threads` has Zeron wake the agent whenever one of the chats it lists stops on a question or a tool approval, even after the agent's turn has ended. The wake names the chat and quotes the questions. A chat can watch up to 50 others. A chat's delegated tasks wake it this way without being listed, and so do a Mission Control lead's step threads.

`respond_to_input` answers the question or permission request another chat is stopped on. It grants a tool approval only when the answering chat runs at **Full access**; otherwise the agent asks you or leaves it to you. Computer use consent is always yours to give, and an agent never answers its own chat's question. You see the same question in the asking chat, and if you answer first, the agent finds nothing left to answer. See [When an agent asks](zeron://docs/agent-questions).

## Stop, rename, archive and settle

- `interrupt_thread` stops another chat's running turn, as its stop button does. An agent never stops its own chat this way.
- `set_thread_title` renames a chat, the agent's own included.
- `set_thread_archived` archives or unarchives one chat or several. A working chat cannot be archived, and archiving one never archives others.
- `set_thread_settled` moves chats to the **Settled** shelf of the sidebar's Inbox view, as **Settle** does, or brings them back. A chat that is working or waiting on a question cannot be settled, and a settled chat comes back with its next message. See [Sessions in the sidebar](zeron://docs/sessions-sidebar).

## Point an agent at a chat

Three places hand an agent a chat to read rather than a copy of it, and the agent reads it with `read_thread`:

- **@ mentions** of threads in the composer insert a link the agent opens. See [Commands and mentions](zeron://docs/commands-and-mentions).
- **Hand off to project** starts a thread in a project with a link to the source chat. When the new chat's agent has `read_thread` off, it gets a text copy of the conversation instead. See [Hand off a thread](zeron://docs/handoff).
- **Reads** on a Mission Control task lists the chats its lead reads before it plans. The field turns off when the lead's tools leave out `read_thread`. See [Mission Control](zeron://docs/mission-control).

## When the Supervisor uses these tools

The [Supervisor](zeron://docs/supervisor) reaches leads and their step threads through the same tools, within the switches under **Authority** on its page:

- Messaging a chat and answering its questions follow **Message a Lead or Peer directly**.
- Starting a chat with `create_thread`, like delegating a task, follows **Start a new mission**, which is off by default.
- It never stops a chat's turn with `interrupt_thread`. A step that blocks a shared resource is paused instead, which needs a contention Zeron verified.

## Limits

- An agent reaches only chats on the same device as its own.
- Messages, new chats, stops and answers to other chats are capped at 16 per turn.
- An agent can keep up to 8 chats it started open at once; archive one to start another. A Mission Control lead's step threads do not count.
- A chat can watch up to 50 other chats.

See [Zeron tools for agents](zeron://docs/agent-tools) for the limits that apply to every tool.

## Turn these tools off

The **Threads** group in [Settings > Tools](zeron://settings/tools) has a switch for the whole group and one for each tool. With `read_thread` off, **Hand off to project** sends a text copy, and the **Reads** field of a Mission Control task turns off.

> To turn thread tools off for only some chats, set the **Tools** field of an [agent profile](zeron://docs/agent-profiles).
