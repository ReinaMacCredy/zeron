The composer sits at the bottom of every open session on the iPhone, and on the **New session** canvas. It works like the [composer](zeron://docs/composer) on your computer: what you pick here is saved with the chat, and the agent runs on the computer that hosts the session.

![A staged quote above the composer's chips, and the Start mission menu](ios-composer.jpg)

## Writing a message

The field reads "Message" followed by the agent's name. With a keyboard attached, Cmd+Return sends.

- **+** opens the attach card in place of the keyboard: **Camera**, **Photos**, **Files**, and **Paste image** when your clipboard holds an image. **Photos** shows your recent photos; select several and tap **Add**, or open **All Photos**. Tap **+** again, or the text, to get the keyboard back.
- A long paste becomes a **Pasted text** tile instead of filling the field. After you send, tap its pill in the transcript to read the text.
- Touch and hold text in the transcript and choose **Quote in reply** to stage it as a quote above the field.
- Type `/` for commands and `$` for skills. Both lists open above the keyboard and match the ones on your computer for every agent, Zeron's own commands included (their descriptions start with **Zeron:**); those run on the phone. Commands that only run in Claude Code's terminal sit at the bottom with a **Terminal** tag. See [Commands and mentions](zeron://docs/commands-and-mentions).
- In a Claude Code chat, `/config` opens Claude Code's settings as a sheet, and `/context`, `/usage`, `/model` and `/mcp` answer with the same cards as on your computer, without starting a turn. See [Claude Code's commands](zeron://docs/commands-and-mentions).
- Type `@` to mention a thread or a file. Threads come first, each with its status, then files from the project.
- When Zeron predicts your next message, it shows in grey in an empty field with **Accept**. See [Composer predictions](zeron://docs/composer).

![The / list on iPhone, and Claude Code's terminal-only commands at the bottom](ios-slash-popup.jpg)

![Claude Code's settings sheet, the /context card and the summary row on iPhone](ios-claude-config.jpg)

## The chips

The chips under the field change this chat's settings.

- **Start mission** comes first. Turn it on to send the message as a [mission](zeron://docs/mission-control) instead (see below).
- **Model** reads the agent profile and the model. A dot after the profile name means you changed the profile's settings in this chat. The menu opens on your [agent profiles](zeron://docs/agent-profiles): apply one, **Revert** or **Save** an edited one, or choose **No profile**. Below them come the models of the current provider and of each provider instance; an instance that fails to load shows why, with **Retry**. On the **New session** canvas, each model shows its agent's mark, and a profile applies to the draft once its Mac is online. Switching models on a chat that already has messages asks **Switch model?** first, and the transcript notes the change.
- **Effort** offers **Low** to **Max**, plus **Fast** where the model has it. See [Models and reasoning effort](zeron://docs/models-and-effort).
- **Approve agent actions** sets the permission mode: **Ask for approval**, **Approve for me**, or **Full access**. It offers only the modes the agent supports.
- **Context** shows how full the context is, such as "92% context". It opens **Context and stats**.

![The permission menu, Context and stats, and the Hand off thread menu](ios-composer-menus.jpg)

## Context and stats

**Context and stats** has:

- **Long context**, with the window sizes the model offers and a warning before the context grows past 200K.
- **Idle compaction**: when the prompt cache expires and when the chat compacts. See [Compaction](zeron://docs/compaction).
- The session's cost and tokens (input, cache read, cache write, output), estimated from public API prices.
- **Session statistics**: LLM time, tool time, time to first token, tokens per second, the cache countdown and the cache hit rate.

**Compact Context** in the session's **…** menu, or `/compact`, compacts now; a **Compacting** pill shows while it runs, then the transcript notes how many tokens it went from and to. When you open a large chat that sat idle, a pill offers to compact it, with **Compact** or **Keep full history**.

## While the agent works

A message you send while the agent is working is queued for the next turn or steers the running turn, as set in **Settings** > **Composer**. Agents that cannot be steered say so and queue it instead.

Touch and hold the send button for **While the agent works**: **Queue for next turn**, **Steer now** (deliver into the running turn), or **Stop and send**. With nothing typed, the button stops the run (**Stop response**). When the turn only waits on background agents, the next message goes straight through.

Queued messages wait in a list above the composer. Tap the arrow on one to send it now, or open its **…** menu to **Edit**, **Move Up**, **Move Down**, or **Remove** it. If another device is editing that message, or it was just sent, the phone says **Can't edit right now**.

## Agents, side chats and hand-offs

The session's **…** menu has:

- **Agents**: the subagents and background shells this chat started, with their status. **Stop** ends one; a shell shows its live output. Session rows also name background work, for example "1 shell" in place of **Working**.
- **New sidechat**: a [side chat](zeron://docs/side-chat) that reads this chat without changing it. Sidechats expire after 24 hours. `/btw` and `/sidechat` start one too.
- **Hand off thread**: continue in this thread with another agent and model, in a new thread, or hand it to a project. See [Hand off a thread](zeron://docs/handoff).

While a chat runs subagents or background shells, a pill above the composer counts them, such as "2 of 3 working · 1 shell". Tap it for a card with each run, its state and how long it has run. **Stop** ends one; tap a row to open its output, its review findings or its chat.

![The subagents pill, and its roster card](ios-agents-pill.jpg)

## Missions from the composer

With **Start mission** on, a second chip picks **plan.md** or **Steps only** and holds **Review plan first**. Send starts the mission and opens it in **Tasks** (see [Tasks and missions on iPhone](zeron://docs/mobile-tasks)).

In an open chat the field reads "Turn this chat into a mission: what should the team build?". **Settings** > **In a chat, Mission Control** decides what happens: **This chat leads** makes the chat the mission's lead, and **New mission reads this chat** starts a new lead that reads it.

## Answering the agent

When the agent asks you something, a question panel opens at the composer. Pick an option, or type your own answer in **Other…**. With several questions, step through them with **Back** and **Next**, then tap **Submit**. See [When an agent asks you something](zeron://docs/agent-questions).
