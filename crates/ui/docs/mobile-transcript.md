Tap a session on the iPhone to open its transcript. It shows the same rows as the [conversation](zeron://docs/conversation) on your computer, sized for the phone. Everything here is read from the computer that runs the session.

![Delegate chips, and a usage limit card](ios-transcript.jpg)

## Reading

- Each settled turn folds its work under **Worked for** and the time it took. Tap it to open the tool calls and thoughts. Thoughts show in full.
- While the agent works, the working row shows what it is doing, how long it has run, and its live speed in tok/s.
- Your own messages render their Markdown and math, so a list or a table you sent reads as one.
- Lines across the transcript mark events such as **Context compacted**, **Model changed**, **Host restarted** and **Resumed**. A hand-off or a switch to another provider shows a "Continued from" line; tap it to read what was handed over.
- A classifier row names its model, how long it took and its verdict; tap it for its card. While the agent waits on compaction or the classifier, the working row says so, even on an idle chat.
- Pages an agent renders with `html_render` show inline in the transcript; their links open outside the app.
- A skill's name shows in violet; tap it to read its SKILL.md.
- Messages carrying a pull request's context, or review comments, show a pill such as **PR context**; tap it to read what was attached.
- When you scroll up, **Jump to latest** takes you back to the bottom.

## Other agents in the transcript

- A `delegate_task` call shows as a chip with the agent's brand mark, its role and model, and whether it was **routed**, **fell back**, or **chosen by agent**. Tap the badge for how it was routed; tap the arrow to open the child thread. See [Delegation](zeron://docs/delegation).
- Subagent rows carry the model maker's mark, or else the chat's agent mark, or else a bot.
- **Started thread** rows name a thread the agent created, with **Open**.
- Messages from another thread show who sent them, and tapping the name opens that thread. Handback quotes from finished steps fold under their message, and a queued message from another thread names its sender.
- **Agents** in the session's **…** menu lists the subagents and background shells still running (see [The composer on iPhone](zeron://docs/mobile-composer)).

## Copy, fork and rewind

Touch and hold a message for **Copy**, **Copy Message** and **Select Text**, then:

- **Fork chat from here**, on a reply, starts a new chat with the conversation up to that reply. This chat stays as it is.
- **Rewind to here**, on one of your messages, removes that message and everything after it and puts it back in the composer. Files on disk stay as they are. Stop a running turn before you rewind.

Both ask before they act. See [Forking and rewinding](zeron://docs/conversation).

![Fork chat from here, Rewind to here, and Session info](ios-transcript-branch.jpg)

## Usage limits and resuming

When an agent runs out of usage, a **Usage limit** card says when the limit resets and offers to switch to another agent now or wait for the reset, with **Switch now** and **Cancel**. **Open chat** opens the chat it belongs to. A pill above the composer says when the chat resumes. Whether chats resume on their own after a reset is the **Limits** switch on your computer's settings page (see [Settings on iPhone](zeron://docs/mobile-settings)). See [Usage](zeron://docs/usage).

A turn that stopped or failed can be resumed: with an empty composer, the send button turns into a play button that resumes it. The transcript then shows a **Resumed** line.

## Session info

**Session Info** in the session's **…** menu shows:

- **Usage**: the account's usage meters, such as the 5-hour window.
- **Sources**: the images and files sent in the chat.

## The session's … menu

From top to bottom: for a mission thread, **Open mission page**, **Brief** and **Watched by the Supervisor**; **Stop Computer Use** while the agent is using apps; **Pin** or **Unpin**; **Session Info**; **Terminal** and **Project Actions** (see [Devices and computer use on iPhone](zeron://docs/mobile-devices)); **Copy Transcript**; **Agents**, **New sidechat**, **Hand off thread** and **Compact Context**; **Archive**.

A thread that belongs to a mission shows a strip at the top with the mission, its step and its progress. Tap it to open the mission's page.
