Zeron fork: merge upstream 0.3.4, build the Auto access mode, update tool text and What's New, rework the composer prediction keys, add Run in terminal to code blocks. Six stages, in order, one PR.

Repo: ReinaMacCredy/zeron-zgx-port (private). Start from branch
`handoff/upstream-0.3.4`. It is the fork's main plus one commit adding
`.handoff/` (read every file there first; drop that folder in your last
commit). Work on a new branch `integrate/upstream-0.3.4`; push only that
branch, only to this repo. Never push to zeronsh/zeron, zeronsh/zgx or
ReinaMacCredy/zeron; never push tags.

House rules (all stages):
- Tests use mock models only (ZERON_HARNESS=mock where a test reads it).
  Never call a real model.
- Never `cargo fmt`; never rustfmt anything under apps/zgx. Format only Rust
  files you changed, through stdin: `rustfmt --edition 2024 < f > f.tmp && mv f.tmp f`.
  Several big files are not rustfmt-clean at HEAD: in those, format only your
  own hunks by hand.
- No code comments except a one-line why a reader could not recover.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Prefer extending a nearby test over a new fixture; add a test only for
  behavior nothing covers.

## Stage 1: merge upstream zeronsh/zeron 2a9e2f987 (version 0.3.4)

    git remote add upstream https://github.com/zeronsh/zeron.git && git fetch upstream
    git merge --no-ff 2a9e2f9871de34ab880d14fc2d3b2204cc4efc4f

Upstream brings 42 commits: tool result pictures as thumbnails that open the
lightbox (#1186, #1103), a Codex chat's `/` lists Codex's commands and skills
(#1403), Windows terminal tabs (PowerShell, Ctrl+C, ConPTY), setup without an
account opens the main window (0.3.3), the edge relay config, Windows test
fixes.

A trial merge gave 29 conflicts. Rules:
- apps/zgx/Cargo.toml and apps/zgx/Cargo.lock: keep the fork's
  `https://github.com/ReinaMacCredy/zgx` URL for every zgx crate, at rev
  `6253134daff26ea7dd3d0f0e9d97f4a249900105` (it already contains upstream's
  0ca78b2cb). Keep the fork's `cutover` feature. Let cargo regenerate the
  lock (e.g. `cargo update -p zgx --precise <sha>` in apps/zgx), not by hand.
- crates/mcp/src/transcript.rs and crates/ui/src/shell/session_lanes.rs: the
  fork deleted them; keep them deleted (upstream only added
  `preview_paths: Vec::new()` there).
- Everything else: keep both sides. The fork added `action:
  Option<ApprovalAction>` to `UserInputQuestion` (crates/proto/src/agent.rs):
  every literal upstream adds needs `action: None,`. Upstream added
  `preview_paths` to tool parts: every fork literal needs it.
- apps/zgx/ui/ds/activity.luau and apps/zgx/src/chat.rs: keep the fork's
  thoughts open/fold as one persisted choice (`THOUGHTS_OPEN`) and the
  advisor tally (`CallKind::Advisor`, "consulted advisor"); take upstream's
  tool pictures.
- apps/zgx/ui/templates/zeron-v2/**: take upstream's, then reapply any
  fork-only line the conflict shows on the fork side.
- crates/engine/src/sessions.rs, crates/harness/src/mock.rs,
  crates/engine/tests/e2e.rs: the fork's newest work (agent approvals,
  `.handoff/agent-approvals/report-A.md`) lives here; keep all of it.
- Version: take upstream's 0.3.4.

Verify, then commit the merge:
- `cargo check --all-targets -p zeron-proto -p zeron-doc -p zeron-harness -p zeron-engine -p zeron-mcp -p zeron-client -p zeron-ui -p zeron-mobile`
- `cargo test -p zeron-doc`, `-p zeron-harness`, `-p zeron-engine`,
  `-p zeron-client`, `-p zeron-mcp --lib`; `cargo test -p zeron-ui --lib -- --test-threads=1` if the machine can build gpui.
- `cargo test -p zeron-engine --test agent_approvals` must stay green.
- apps/zgx: `cargo build`, then the zgx CLI's `test ui` (build the CLI from
  https://github.com/ReinaMacCredy/zgx at the pinned rev; apps/zgx/README.md
  says how; its filter matches test names, not file names). Two tests failed
  before the merge and are known: settings_archived "thousands archived" and
  tasks_page "a thousand tasks". Any other failure is yours to fix.

## Stage 2: Auto access mode (U4)

Read `.handoff/agent-approvals/SPEC.md` in full ("U4 Auto access mode", and
"Facts" for anchors) and `report-A.md` for how U1-U3 were built (the live
allowance on `RunHandle`, `approval::covered`, `least_covering`, the
`request_input` chokepoint in `RunSetup::new`). Build U4 as SPEC says:
- `AccessMode::AutoReview` between AutoAcceptEdits and FullAccess, wire name
  `auto-review`, label "Auto". Check every exhaustive match and `ALL` user.
- Claude Code launches with `--permission-mode auto`; approvals that still
  reach Zeron go to the engine classifier.
- Engine classifier in the chokepoint: one-shot on the chat's harness with
  `cheapest_model` (see `titles::one_shot_target` / `run_one_shot`), no
  tools, short timeout, ALLOW or ASK only, fail closed to asking the person,
  and an ALLOW recorded in the transcript as answered "by Auto".
- Tests: drive the classifier through the mock harness only (a mock
  one-shot that answers ALLOW, ASK, garbage, and times out). Never a real
  model.
- Older builds: check what an unknown `AccessMode` does in `ChatConfig`
  deserialization (iOS, cloud engines on upstream). If it breaks a chat's
  sync, make the read lenient or keep Auto off for cloud chats; say which.
- Pickers: the zgx composer access chip (apps/zgx/ui) offers Auto where the
  harness lists it; iOS only if it matches exhaustively.

## Stage 3: tool text, guidance, What's New, docs (U5)

- MCP tool descriptions in crates/mcp/src/lib.rs: set_thread (`access`),
  respond_to_input (`scope`), and the access enums of delegate_task and
  agent_profiles (Auto). Short and accurate.
- `routing/guidance.rs` (both copies) and `crates/engine/src/doc_host_delegation.rs` ~228.
- Re-measure `crates/engine/tests/agent_context_budget.json` (the test says
  how; it currently fails by +533 bytes from U1-U3's text).
- What's New (`crates/ui/whats_new.toml`; skill: `.handoff/zeron-whats-new-docs-SKILL.md`):
  the release is now 0.3.4. Turn the existing 0.3.2 fork entry into the 0.3.4
  entry, add `upstream-0-3-3` and `upstream-0-3-4` features summarizing those
  releases, and add improvement lines for: a lead can raise its child's
  access and approve for the turn; the Auto access mode; the rail's account
  button and animated update button (zgx); thoughts Show all / Show fewer
  for every chat; "consulted advisor" in a work group's summary.
  Run `cargo test -p zeron-docs`, `cargo test -p zeron-ui --lib whats_new`.
- A docs article for access modes (crates/ui/docs/, index.toml, BODIES) if
  none covers them; text only, no new screenshots.

## Stage 4: the composer's prediction, taken from anywhere, with its keys shown (zgx)

Today (apps/zgx/ui/ds/composer.luau ~2339-2430, ~2536, ~2654): a prediction
shows as the empty composer's placeholder, or as a ghost after the caret when
the draft starts with it; Tab accepts it, but only while the composer has the
keys. `screens/chat/typing.luau` already sends a key typed with nothing
focused to the composer (gpui's `typing_target`).

Wanted:
- With a prediction showing and the composer not focused (nothing else
  taking the keys: not a text field, menu, dialog, terminal or editor),
  pressing Tab or Right arrow focuses the composer and fills it with the
  whole prediction, caret at its end. Focused, Tab and Right arrow (the
  latter only with the caret at the draft's end and nothing selected) do the
  same as Tab does today. Right arrow anywhere else keeps moving the caret.
- Beside the prediction (after the placeholder text or the ghost, on the same
  line, in the same tertiary color), a small keycap hint as T3 Code shows
  it: a `Tab` keycap and a `→` keycap, using zgx's own `ds/kbd.luau`. It is
  aria-hidden, takes no pointer, hides with the prediction, and never wraps
  onto its own line (a long prediction truncates before it).
- Settings → Predictions off: no hint, no key taken.
- Tests: extend the existing composer prediction tests (search ui/tests for
  `prediction`): unfocused Tab fills and focuses, unfocused Right arrow fills
  and focuses, Right arrow mid-draft only moves the caret, a focused text
  field elsewhere keeps its Tab, and the hint shows and hides with the
  prediction.

## Stage 5: Run in terminal on a chat's code block (zgx)

Today a code block in a reply (apps/zgx/ui/ds/markdown.luau ~1219: the strip
with its language, the fit/wrap toggle and Copy) can only be copied; pasting
a long command into a terminal often breaks it (a wrapped line pasted out of
order, a stray character).

Wanted:
- A "Run in terminal" button in the strip, beside Copy (same size, tone and
  Tab stop order: fit, Run, Copy; a terminal glyph from zgx's icons, tooltip
  "Run in terminal"), on blocks whose language is a shell (bash, sh, zsh,
  shell, console, fish, powershell, pwsh, ps1) or that have no language or
  `text`.
- Pressed: open the chat's terminal (the panel/dock terminal for the chat's
  project folder; see screens/panel/terminal.luau and terminal_api.luau,
  `write`), show it, and write the block's text followed by Enter, so it
  runs at once. `console` blocks drop a leading `$ ` / `% ` / `❯ ` prompt
  per line. Multi-line blocks are written as they are, one Enter at the end.
- If the chat's terminal is busy (a program running, not at its prompt) or
  none exists, open a new terminal tab in the chat's folder and run it
  there; never type into a running program.
- A chat on another device or Cloud: its own device's terminal if the
  terminal API reaches it, else the button is absent.
- Also check Copy itself: copying a long single-line command from a wrapped
  block must give the exact source text (no soft-wrap newlines, no lost or
  added characters). Fix it if it doesn't.
- Tests: extend the markdown code block tests: the button shows for shell /
  text / unlabeled blocks and not for e.g. rust or json; pressing it writes
  the exact text plus Enter to a terminal (fixture-backed host seam, no real
  shell); a busy terminal gets a new tab; Copy of a wrapped block is exact.

## Stage 6: PR

Remove `.handoff/`, push `integrate/upstream-0.3.4`, open a PR against
`handoff/upstream-0.3.4` in this private repo. Body: each conflict and how
it was resolved; U4's design choices and the older-build finding; every
test command with pass/fail counts; anything left or unverified. Do not
merge it.
