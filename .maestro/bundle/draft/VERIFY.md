# VERIFY — draft

Scenarios point at work acceptance/claims; do not restate them.

Rig: worktree `.maestro/worktree/draft` (base 8b6b7ab6), own target dir;
`cargo test -p zeron-ui --lib -- --test-threads=1`; Zeron Dev 8 bundle
(`target/macos-dev8`, port 49790, mock harness) seeded over IPC with
`rpc_probe` and driven with capture knobs (see SPEC derived artifacts).

| # | Scenario | Work / acceptance | Result |
|---|----------|-------------------|--------|
| 1 | UI lib suite, single-threaded | w27 (no regression) | 1048 passed, 0 failed (2026-09-17); one prior test updated: the last tab now closes onto a draft tab (d18/d20) instead of an empty strip |
| 2 | `draft_store` round-trips prompt + attachment path; empty entries not written; orphan images removed; missing image drops the attachment not the draft; corrupt file starts empty; sessions persist only with content | w28 red tests | 5 unit tests pass |
| 3 | Typed draft survives a restart | w28 A | `ZERON_DRAFT_TEXT` boot draft → `composer-drafts.json` holds its entry + session 7s later; relaunch without the knob shows it as a sidebar row and in the switcher (`capture-restored-rows-and-pen.png`, `capture-switcher-draft-cards.png`) |
| 4 | Unsent pen on a session row | w28 B | seeded entry for `chat-a` → amber pen before the title (`capture-restored-rows-and-pen.png`) |
| 5 | Drafts are tabs; ⌘N reuses an empty draft and opens a second beside an invested one; leaving an empty draft discards it; closing an invested draft's tab keeps its row; discard drops session + text | w29 C | `drafts_are_tabs_and_empty_ones_die_when_left` passes |
| 6 | Promotion keeps the id: selecting the chat minted with the draft's id removes the session, re-keys nothing, keeps the tab | w29 C | `a_draft_promotes_into_the_chat_that_carries_its_id` passes |
| 7 | Open draft's row is a frozen snapshot; the switcher tour opening a draft shows its row + restored text + restored ref (`main`) | w29 C, d21 | `capture-switcher-draft-cards.png` (row "add draft rows…" at top while open, composer text and ref restored) |
| 8 | Switcher cards for drafts: typed text where a session shows its last message; thumbnail once viewed | d18 | `capture-switcher-draft-cards.png` |
| 9 | Background start: Mod+Enter on a draft mints the session without selecting it, canvas moves to a fresh draft, started session appears Working in the sidebar and keeps a tab, next draft carries branch + worktree mode | w30 D, d23 | `background_start_keeps_the_tab_and_mints_the_next_draft` passes; `capture-background-start.png`; after restart the started chat carries the prompt as its transcript |
| 10 | Anti-goal: model/mode-only draft has no row and no disk entry | SPEC anti-goals | `DraftEntry::has_content` ignores picks; `sessions_persist_only_with_content` passes |
| 11 | Anti-goal: Loro doc unchanged; discard emits no RPC | SPEC anti-goals | no proto/engine files in the diff (`git diff --stat`: ui crate only); `discard_draft` touches store + state only |
| 12 | Anti-goal: send wire shape unchanged | SPEC anti-goals | `send()` builds the same `createChat`/`Run` payloads; only the chat id source changed (draft id instead of a fresh uuid) |

Known base failures on this Mac (harness codex pagination, shell_env hang
fallback, engine generated_image prefix) were not run; the UI lib suite is
the changed surface.
