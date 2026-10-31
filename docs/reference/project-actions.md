# Project Actions

Project Actions are named shell commands attached to a project. They run from the **Actions** section of the selected chat's Live state column (the Sidecar), and one saved Action can optionally run when Zeron creates a new worktree. The Cmd+K palette offers only imports and **Add action…**; it never runs an Action.

## Trust and storage

Actions are private configuration on the device that owns the project. Zeron stores them in the active engine profile's `project-actions.json`; they are not written to the workspace registry, session documents, or the edge service.

Opening a repository never authorizes a command. A repository may offer Actions through `zeron.json`, but each candidate must be explicitly imported before it can be run or selected as setup.

For a remote project, list, edit, delete, and run requests are sent to the owning device through `targetDeviceId`. The viewing device never resolves the remote path or falls back to running the command locally when the owner is offline.

## `zeron.json`

Place `zeron.json` at the exact project root to offer version-controlled imports:

```json
{
  "actions": [
    {
      "name": "Dev server",
      "command": "pnpm dev",
      "icon": "play",
      "runOnWorktreeCreate": false
    }
  ]
}
```

The supported icons are `play`, `test`, `lint`, `configure`, `build`, and `debug`. Unknown fields, malformed JSON, invalid Actions, or more than 50 entries invalidate the whole file. An import is hidden when a saved Action has the same exact command or the same case-insensitive name.

## Limits and normalization

A project can store at most 50 Actions. Names are trimmed, must be non-empty, and may contain at most 80 Unicode characters. Commands are trimmed, must be non-empty, and may contain at most 16 KiB of UTF-8 data.

The owning engine generates a stable slug id of at most 96 bytes and adds a deterministic numeric suffix on collision. Saving an Action with `runOnWorktreeCreate` enabled atomically disables that flag on every other Action in the project.

## Execution

Every invocation opens a fresh managed terminal in the chat's bottom terminal dock, in a tab named after the Action. On Unix, Zeron stores the exact saved command in a private temporary script on the owning device and sends a short instruction to source it (`. "$ZERON_ACTION_SCRIPT"`, or `source` in fish) in the user's interactive login shell. This preserves long lines and multiline commands across shell initialization without passing them through the terminal's limited input-line buffer. The script is removed when the shell exits or the terminal is closed. Zeron does not wrap the command in `sh -c` or reuse a terminal whose foreground state is unknown.

The script installs a one-shot hook for the shell's next prompt before the command: `precmd` in zsh, `PROMPT_COMMAND` in bash, and the `fish_prompt` event in fish. When the command ends and the prompt returns, including after Ctrl-C, the hook prints `ESC ] 7502 ; <status> BEL` and removes itself. Other shells print the same marker right after the command. The desktop reads that marker from the terminal output, even when it arrives split across chunks, and the Action's row reads Done for status 0 and Failed otherwise. The shell stays open, so an Action that exports variables or activates an environment leaves a prepared, usable shell. Windows retains direct command submission to its native terminal, which does not use the Unix canonical line buffer; it installs no hook, so the row stays Running.

The owning engine validates that the Space, Chat, and checkout belong to the same local project before opening the PTY. A manual run uses the chat checkout as its working directory and injects:

```text
ZERON_PROJECT_ROOT=<canonical project root>
ZERON_WORKTREE_PATH=<canonical chat worktree, only outside the main checkout>
```

The terminal output is replayable, so output produced before the desktop subscribes is still displayed. Subscribe, resize, write, and close requests retain the terminal's owning `targetDeviceId`.

Zeron records the last successfully started Action for each project in viewport-local UI settings (`lastProjectActionBySpaceId`) and drops the record when that Action is deleted. No current surface reads it: the title-bar Action segment that used it is gone.

## Worktree setup

Only creation of a new worktree can start the setup Action. Reusing an existing worktree and using the main checkout do not run setup again.

For desktop sends, the worktree directive rides the durable queued `Run` command. The owning engine creates the worktree and starts setup while draining that command, before dispatching the first agent turn. Setup runs with the new worktree as its cwd and always receives both `ZERON_PROJECT_ROOT` and `ZERON_WORKTREE_PATH`.

The queue reply is not held open while the host creates the worktree. Desktop polls a short-lived, command-scoped handoff to attach the already-open setup terminal when it becomes available. A lost relay reply therefore cannot leave the composer stuck on `Sending…` while the agent runs remotely.

Direct `CreateWorktree` callers, including iOS, retain the same host-side setup behavior and receive the optional setup terminal in the RPC outcome.

Desktop attaches the returned terminal to the bottom terminal drawer of the newly minted chat, even if another chat becomes selected while the request is in flight. Manual Actions use the same bottom drawer. iOS starts setup through the same host-side RPC but does not add a terminal surface in this release.

A failure to open or initialize the setup terminal is returned as `setupError`. The worktree remains available and the first agent turn continues; desktop presents the error as a non-blocking notice.

`CreateWorktree` and `RunRequest.worktree` remain additive and wire-compatible across versions. Callers that omit `spaceId` receive worktree creation without setup, legacy clients ignore optional fields, and desktop silently skips the handoff when connected to an older host.

## Out of scope

Project Actions do not define global keybindings. Preview URLs and automatic browser opening are also intentionally absent until Zeron has a remote-capable browser and tunneling design.
