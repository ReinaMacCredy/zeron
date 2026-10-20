Three characters open menus in the composer: `/` for commands, `$` for skills, and `@` to mention files and other threads. Use Up and Down to move through a menu, Enter or Tab to pick a row, and Escape to close it.

## `/` commands

Type `/` to list the agent's built-in commands. They come from the agent itself, so the list differs between Claude, Codex, and the others. Zeron adds a few of its own:

| Command | What it does |
| --- | --- |
| `/compact` | Free up context by summarizing the conversation. |
| `/btw` or `/sidechat` | Ask about this chat on the side, without touching it. |
| `/computer-use` | Use Zeron Computer for this request only. Offered for Claude Code, Codex, Devin, Grok, and Hermes. |
| `/model` | Choose agent, model, and reasoning. |
| `/new` | Start a new conversation. |
| `/resume` | Search and open conversations (the Cmd+K palette). |
| `/settings` | Open settings, on the page you used last. |

Inside a chat you also get `/diff` (open changes), `/files` (open project files), `/terminal`, `/rename` (rename this conversation), and `/stop` (stop the active run). `/diff`, `/files`, and `/terminal` each open a tab in the [right pane](zeron://docs/right-pane); `/terminal` gives you a terminal tab there, not the Cmd+J drawer.

`/model`, `/new`, `/resume`, `/settings`, and the five chat commands run in Zeron and are never sent to the agent; their menu descriptions start with **Zeron:**. If the agent has a command with one of these names, Zeron's shows as `/zeron:<name>`. `/compact`, `/btw`, and `/sidechat` work differently: when the agent lists its own command with that name, the menu shows only the agent's row, but Zeron still handles what you type. Zeron's `/computer-use` replaces any `/computer-use` the agent has.

Type a request after `/computer-use`. With nothing after it, the message is not sent and the composer shows **Add a request after /computer-use.**

## `$` skills

Type `$` to pick one of the skills the agent finds for this project, such as the skill folders in `.claude/skills` or `.agents/skills`. Each row shows the skill's name and description, with the name in Title Case (`my-skill` reads **My Skill**). Zeron passes your pick to the agent in the form that agent expects.

Two switches per agent control this, under **Completion** when you expand an agent in [Settings > Providers](zeron://settings/providers):

- **Use $ for skills**: type `$` in the composer to pick a skill.
- **Separate / commands**: keep skills out of the `/` menu.

Both are on by default. Turn off **Use $ for skills** and `$` is plain text again.

When Claude Code loads a skill, its row in the transcript names the skill in violet. Click the name to open the skill's `SKILL.md` in the right pane.

## `@` mentions

Type `@` to open one menu with two sections:

- **Threads** lists up to six other threads on the same device, with their status (**Working**, **Input**, **Failed**, **Done**) and project. Threads in this chat's project come first. Type to match a title. Picking one inserts a link the agent can open with Zeron's `read_thread` tool (see [Agents and other threads](zeron://docs/threads)).
- **Files** lists files and folders in the project, plus anything you have attached. Picking one inserts a link to its path.

You can also drag a file from the file tree, or a session from the sidebar (**Drop to mention**), onto the conversation to insert the same mention.

> `/btw` runs a side question without disturbing the thread. Cmd+; toggles the sidechat too.
