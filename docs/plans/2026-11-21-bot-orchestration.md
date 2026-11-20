# Bot orchestration and a clean "Bot" layout

Goal: run Zeron the way a Devin Bot runs: one persistent lead you talk to, which
files every piece of work as a ticket, hands each ticket to a child thread, keeps
its own memory, wakes on a schedule, and asks you for what only you can do
through an Inbox of cards. Plus a quiet, Devin-like layout built around that.

Status: plan only. Nothing here is built yet.

## 1. What Zeron already has

Most of the machinery exists; the gap is a persistent identity on top of it and
a layout that puts it first.

| Devin Bot concept | Zeron today | Gap |
| --- | --- | --- |
| Bot (persistent lead conversation) | A **lead** chat: any chat that delegates, plans or starts a thread ([mission.md](../../crates/docs/articles/mission.md)) | A lead is per chat and ad hoc. No single long-lived Bot chat, no identity, no standing instructions across chats. |
| Child sessions | `delegate_task`, `create_thread`, plan steps on threads; limits in Routing ([delegation.md](../../crates/docs/articles/delegation.md), [threads.md](../../crates/docs/articles/threads.md)) | None for the mechanics. Children need a "report a milestone to the Bot" convention. |
| Children wake the Bot | Finished/failed/stopped threads and questions wake the lead; `watch_threads` | None. |
| Board + tickets | **Tasks** page, List/Kanban, dependencies, `tasks` MCP tool, a task tracks its chat (`crates/proto/src/entities.rs` `Task`, `crates/doc/src/registry/tasks.rs`) | Status is derived (`ToDo/Working/NeedsYou/Done`); no **In review**, no claim by thread, no folder, no status note, no PR link. |
| Inbox of asks | Cmd+I tray (failed threads, tasks waiting), **Waiting** pill (questions, plans), Inbox history page ([inbox.md](../../crates/docs/articles/inbox.md)) | Only questions an agent is blocked on. No cards an agent files on purpose (approve / decide / review PR / FYI), no priority, no answer routed back to the Bot. |
| Memory | Agent profiles' instructions; project `AGENTS.md` | No Bot memory folder read at every turn and shared with children. |
| Schedules / self-wake | Claude's in-process `ScheduleWakeup` / `CronCreate` are tracked, not owned (`crates/engine/src/self_schedule.rs`); limit resumes (`resume_schedule.rs`) | No engine-owned schedule that wakes a chat, survives restarts, and works for every harness. No promises with a due time. |
| Playbooks | Playbooks ([playbooks.md](../../crates/docs/articles/playbooks.md)) | None. A schedule can start one. |
| Layout | Templates: a whole UI as a Luau folder laid over the default (`apps/zgx/docs/templates.md`, `apps/zgx/ui/templates/zeron-v2/`) | A Bot-first template. |

## 2. Engine work

### 2.1 Bot

- New registry row `Bot { id, name, space_id, chat_id, agent, model, effort, profile_id, memory_dir, created_at }`.
  One Bot per space by default; more are allowed.
- The Bot's chat is an ordinary chat flagged `bot_id`. It is never archived or
  settled, never counted against **Delegated tasks per chat** of its children's
  children, and compacts as any chat does (its memory, not its transcript,
  carries what lasts).
- Every Bot turn's prompt gets a preamble: the Bot's role, `MEMORY.md`, the open
  Inbox cards, and the board's open tickets (titles, status, claiming thread).
  Same mechanism as `mission_brief` / `lead_brief` in `crates/engine/src/rpc/tasks.rs`.
- RPC: `CreateBot`, `UpdateBot`, `ListBots`, `DeleteBot`. MCP: none (agents do
  not create Bots).

### 2.2 Memory

- `memory_dir` defaults to `{data_dir}/bots/<bot_id>/memory/`, a git repo with
  `MEMORY.md` plus topic files, same entry format as Devin's
  (`[source: <chat>; added: <date>; confidence: low|medium|high]`).
- MCP tool `memory` (`read`, `write`, `sync`): the engine commits each write, so
  two threads writing at once merge instead of clobbering.
- Children started from a Bot ticket get `MEMORY.md` in their first turn and
  the same tool. Only the Bot writes `state/`.
- Synced workspaces: the memory folder replicates as files through the existing
  workspace file path; conflicts surface as a card.

### 2.3 Board (extend Tasks, do not add a second tracker)

Add to `Task`:

| Field | Purpose |
| --- | --- |
| `status_override: Option<ManualStatus>` (`UpNext`, `Working`, `InReview`, `Complete`) | Explicit status; derived status stays the fallback |
| `claimed_by: Option<ChatId>` | The thread working the ticket (today `chat_id` covers one chat; keep it, add claim semantics: claim fails if held) |
| `folder: Option<String>` | Sidebar grouping |
| `status_note: Option<String>` | 1-3 line note shown above the notes |
| `links: Vec<Link>` | PRs, issues; a linked PR merging moves the task to Complete |
| `bot_id: Option<String>` | Which Bot's board |

`tasks` MCP tool gains `dispatch` (create thread + claim + first message, the
existing `start_task` path), `claim`, `release`, `status_note`, `move_folder`.
Kanban columns become Up next / Working / In review / Complete when a Bot owns
the board; ordinary Tasks keep today's columns.

### 2.4 Inbox cards

- New registry table `asks`: `{ id, bot_id, kind: approval|decision|freeform|fyi|pr, title, summary, details, options[], recommended, urgency, effort, priority, subject (pr url | task id | chat id), status: open|on_hold|done|canceled, resolution, engaged_at, created_at }`.
- MCP tool `inbox` (`create`, `update`, `adjust`, `snooze`, `close`, `list`),
  same shape as Devin's so prompts carry over.
- Answering a card (button, option, or a reply in its thread) posts
  `Inbox update: <title>: <answer>` into the Bot's chat, which starts a turn.
- Today's blocking questions and plan approvals (`agent-questions`, plan cards)
  appear as cards too, generated, with `kind` = decision and the request id as
  subject; answering one calls the existing `respond_to_input`. One place for
  everything that needs you.
- Desktop notification and the titlebar pill count open cards.

### 2.5 Schedules and promises

- Engine-owned `schedules` table: `{ id, bot_id | chat_id, cron, tz, prompt | playbook_id, next_at, enabled }`.
  A tick task in the engine (same wall-clock nap loop as `resume_schedule.rs`)
  sends the prompt to the chat at `next_at`, or starts a playbook thread.
  Works headless (`zeron daemon`), which is the point: the VPS keeps the Bot awake.
- Promises: `{ id, task_id, outcome, reply_to, due_at, status }`. Due promises
  and tickets moved to In review / Complete by someone else wake the Bot.
- MCP tool `schedule` (`create`, `list`, `pause`, `delete`) and `promise`
  (`create`, `redate`, `close`).

### 2.6 Child protocol

- A thread started from a ticket gets a short brief: report milestones only (key
  finding, decision needed, PR opened, blocker, done) with
  `send_message_to_thread` to the Bot; keep the ticket's status note current.
- Already delivered: those messages arrive grouped as one card per thread in the
  lead's transcript.

## 3. The clean layout: template `zeron-bot`

A third official template beside Zeron v3 and Zeron v2, laid over the default
like `ui/templates/zeron-v2/` and registered in `apps/zgx/src/plugins/official.rs`.
Reversible by design (View → Use Zeron v3). Reference: Reina's two screen
recordings of the Devin desktop app (2026-11-21); the layout below copies them.

```
┌──┬────────────────────┬──────────────────────────────────────────────┐
│⌂ │ Bots          + ⋯  │                    (◉)                       │
│◉ │ ◉ Code Reaper      │               Code Reaper                    │
│⌕ │   last line…       │               2 need you                     │
│⛨ │ ◉ Chief of Staff • │  Code Reaper                                 │
│⋯ │   last line…       │  ┌──────────────────────┐                    │
│  │                    │  │ Bot bubble (grey)    │  ┌───────────────┐ │
│  │ Needs you 2        │  └──────────────────────┘  │ You (accent)  │ │
│  │ Pull requests 2    │  ┌──────────────────────┐  └───────────────┘ │
│  │  ⇅ #8 Plan: Bot…   │  │ ● Needs you          │                    │
│  │                    │  │ text, links, ⇅ #8    │                    │
│  │ Sessions ⌄     ⇵   │  └──────────────────────┘                    │
│  │ ▣ Bot orchestr. 1  │  ·· Action required: Review the plan         │
│  │   Plan bot orch…   │  ┌────────────────────────────────────────┐  │
│? │   PR ready · ⇅ 1   │  │ Guide Code Reaper while it works…      │  │
│⚙ │ ⌕ Dead code 2      │  │ +  /                  model  🎙  ↑     │  │
│R │   Remove unused…   │  └────────────────────────────────────────┘  │
└──┴────────────────────┴──────────────────────────────────────────────┘
```

- **Rail** (`screens/shell/rail.luau`): a slim icon column. From top to bottom it
  holds Home, Bots, Search, Security/Review, and a ⋯ overflow (Review,
  Automations, Wiki, Apps), with Help, Settings and the avatar at the bottom.
  It replaces the current app rail and its labels.
- **Sidebar, Bots mode** (`screens/shell/sidebar.luau`, `ds/sidebar.luau`):
  - **Bots** header with `+` (new Bot) and `⋯`. Each Bot row has a square avatar,
    its name, the last line it said in muted text, a blue dot when unread, and
    a hover `⋯`.
  - **Needs you N**: the open Inbox cards grouped by kind, for example
    **Pull requests N** with one line per PR (`⇅ #8 title`).
  - **Sessions ⌄** with a filter button: ticket folders (reusing Sections),
    each with an icon and a count. Every ticket is two lines: the title, then a
    status line such as `Working`, `PR ready, waiting for CI · ⇅ 1` or
    `PR is ready · ⇅ 4`, coloured green when ready. A spinner dot marks work
    that is running.
- **Sidebar, Home mode**: **New session**, Recent, and Pinned groups (a parent
  with its child sessions, `☁ 3 minutes ago`, unread dots).
- **Center, Bot chat** (`screens/shell/page.luau`, `screens/chat/`):
  - The Bot's avatar is centred at the top with its name and `N need you` in
    orange; clicking it opens the Inbox.
  - Bot messages are grey bubbles under a small name label, with the avatar
    beside the last bubble. User messages are accent (magenta) bubbles on the
    right. The column is about 480 px wide.
  - When the Bot raises an Inbox card, it appears in the transcript as a
    bubble with an orange `● Needs you` header and inline links (`⇅ zeron#8`).
    A muted `Action required: …` line follows.
  - The composer reads "Guide <Bot> while it works, or press ⌘↵ to queue" and
    has `+`, `/`, model, mic and send controls.
- **Inbox card overlay** (new `screens/inbox/card.luau`, fed by `screens/chat/asks.luau`): clicking `N need you`
  or a card dims the transcript and opens one card at a time. The card shows
  `Needs you 1/N` with ↑/↓, ⋯ and ✕, then the title, the age, a summary of 2 or 3
  lines, the subject card (PR title, `+173 −0`, author), and the actions:
  **Dismiss ⌄** plus the primary button (Merge, Approve, or one per option).
  A disabled action explains itself in a popover. The composer switches to
  "Replying to <card title>", so a typed reply answers that card.
- **Home** (`screens/chat/home.luau`, `home_cards.luau`): a centred brand mark, a composer with Cloud/Local,
  folder and harness pickers, and a **Your Bots** grid of avatar, name and last
  line with **View all →**.
- **New Bot** (new `screens/bot/new.luau`): a two-pane modal. The left pane is
  art with "Your custom Bot"; the right pane asks "What's your goal?" with a
  searchable template list (name plus a one-line purpose, `On your team` for
  existing Bots) and **Continue**. Templates are playbooks with a Bot prompt.
- **No right panel by default.** It opens from the titlebar toggle and keeps
  the existing Changes / Files / Terminal tabs, with Board added as a tab.
- **Look** (`ds/tokens.luau`, `theme.luau`):
  - Colour: a light grey window (`#F7F7F8`), a white content area, and 1 px
    hairlines with no shadows or glass. The accent is magenta for user bubbles
    and avatars' rings; orange marks Needs you, green marks ready, and blue
    dots mark unread.
  - Type and shape: 13 px body and 12 px meta, a 12 px radius on bubbles and
    cards, 8 px on rows, and a 4 px spacing grid.
  - Hidden from the primary layout: home widgets, the usage ring and the
    "agent updates" pill.
- Every screen kept is the default's file; the template overrides only the
  files above, so it stays current as the default changes (the `.base.json`
  staleness guard covers it).

## 4. Delivery, one PR each

| # | PR | Depends on | Proof |
| --- | --- | --- | --- |
| 1 | This plan | — | Review |
| 2 | Bot row, RPC, Bot chat flag, turn preamble | 1 | `cargo test -p zeron-engine --lib bot` |
| 3 | Memory folder + `memory` tool | 2 | engine lib tests: concurrent writes merge |
| 4 | Task fields + `tasks` tool `dispatch/claim/status_note`; In review column | 2 | registry tests, Tasks fixture |
| 5 | `asks` table + `inbox` tool + Inbox update routing; questions as cards | 2 | engine lib tests, MCP contract test |
| 6 | Schedules + promises, headless | 2 | `--test` with a fake clock |
| 7 | `zeron-bot` template: rail, Bots sidebar, Bot chat bubbles, Inbox card overlay, New Bot modal, tokens | 4, 5 | Zeron Dev screenshot, `scripts/ci/test-zgx.sh ui` |
| 8 | iOS: Inbox cards and Bot chat on the phone | 5 | Simulator screenshot |
| 9 | Docs articles: Bot, Inbox cards, Schedules | 2-7 | — |

PRs 3-6 are independent after 2 and can run as parallel threads.

## 5. Decisions taken (change any of them)

1. Extend Tasks into the board rather than add a separate board entity: one
   tracker, Kanban and `tasks` tool reused.
2. Ship the layout as a template, not a change to Zeron v3: reversible, does not
   fight upstream merges from zeronsh/zeron.
3. Inbox cards replace the Cmd+I tray's "Needs you" for Bot spaces and open
   as a centred overlay, not a right-panel tab; the Waiting pill stays for the
   chat in front of you.
4. Memory is a git folder (same format as Devin's), not a SQLite table: readable,
   diffable, and agents already know the format.
5. Schedules are engine-owned so they work for every harness and headless.
6. Several Bots from the start (the sidebar lists them, as in the Devin app),
   each owning one space; Bots sharing a space come later.
