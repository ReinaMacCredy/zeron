A playbook is a saved recipe your agent follows: a markdown file with steps. Starting one opens a new chat with the playbook linked in the composer, so you add what you need and pick any agent and model before you send.

## Start a playbook

- Open the command palette with Cmd+K and pick **Triage an issue**, or a row named **Playbook:** and the playbook's title.
- Or open [Settings > Playbooks](zeron://settings/playbooks) and click **Start** on its row.

A new chat opens. The headline above the composer is the playbook's question, and the composer already holds `Read [<file>.md](<path>) and follow it.` with the cursor on a blank line under it. Write your request there, choose the agent and model, and send. Nothing is sent until you do.

A playbook from a project opens in that project. A playbook for all projects opens on the project you were on, or with no project.

## Triage a Zeron problem

**Triage an issue** is built in. It lets your own agent debug Zeron itself:

1. Pick **Triage an issue** in Cmd+K. The headline reads "What went wrong?" with the Zeron version under it.
2. Describe what went wrong under the link, pick an agent, and send.
3. The agent reads a facts file Zeron wrote for this triage: the version and build, your OS, whether the engine is running, where Zeron keeps its logs, database and settings, and which agent CLIs are installed. It holds no email, token or chat text.
4. Zeron clones its source for your version in the background, so the agent can match log lines and errors to real code.
5. The agent investigates and tells you what it found. It asks before it applies a fix, before it restarts Zeron, and before it posts anything.
6. If the bug is Zeron's, the agent drafts a GitHub issue for you to read first, with emails, tokens, private paths and chat contents left out.

The triage files are in `~/.zeron/triage`, one folder per triage, and the source clones in `~/.zeron/triage/source`.

When the app will not start, run this in a terminal instead:

```sh
zeron triage
```

It writes the same files and starts Claude Code or Codex in the terminal with the same instructions, asking which when both are installed. `zeron triage --dry-run` writes the files and prints the command without starting an agent.

## Write your own

[Settings > Playbooks](zeron://settings/playbooks) lists every playbook with where it comes from.

- **New playbook** (top right) writes a template into `~/.zeron/playbooks` and opens it in your default editor.
- **Edit** opens a playbook's file; **Delete** asks once more, then removes it. The built-in Triage playbook cannot be edited or deleted.
- **Reveal folder** shows `~/.zeron/playbooks` in Finder.

Playbooks in `~/.zeron/playbooks` are offered for every project. To keep one with a project, put it in that project's `.zeron/playbooks` folder; it shows for projects on this Mac.

A playbook starts with frontmatter:

```markdown
---
title: Cut a release
question: Which version are we releasing?
subtitle: Runs the release checklist
---

1. Read the version the person gave.
2. ...
```

`title` names it in Cmd+K and Settings, `question` becomes the headline of the new chat, and `subtitle` (optional) is the line under it. The rest of the file is what the agent follows.

On iPhone, playbooks are not in the command palette yet.
