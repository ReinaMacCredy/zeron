# handback w10 — sidechat UI

## Status

DONE

## Claim

The sidechat UI is wired end to end in `crates/ui`: a `Sidechat` picker row, a
`/sidechat [prompt]` composer command and a footer reopen pill all funnel into
one shell routine that opens (or creates, via `CreateSidechat`) the chat's side
conversation as a `RightSurface::Sidechat` tab hosting its transcript and its
own composer; the sidebar nests a sidechat under its source. No engine, proto,
edge or iOS file was touched.

Scope deviation from SPEC line 19 ("model picker on the sidechat changes only
the sidechat"): the tab's composer hides the pickers footer and the queue
panel instead. Both surfaces read AND write `state.selected_chat`
(`pickers.rs:1569 update_chat_config` → `Mutate setChatConfig`;
`queue.rs` acts on `state.queue`), so leaving them live in the tab would have
reconfigured or drained the SOURCE chat. Per-sidechat model picking needs a
pickers rewire, which the brief excluded.

## Proof

- `cargo test -p zeron-ui --lib sidechat -- --test-threads=1` → 7 passed
  (composer parse/dedupe/pill-label/intercept/nesting-refusal, shell tab
  open+close, sidebar nesting).
- `cargo test -p zeron-ui --lib session_panels_update_tracks_right_surfaces -- --test-threads=1`
  → 1 passed (`RightSurface::Sidechat` round-trips through the panels map).
- `cargo test -p zeron-ui --lib bound_composer -- --test-threads=1` → 1 passed.
- `cargo check -p zeron-ui --tests` → clean.
- Full `cargo test -p zeron-ui --lib -- --test-threads=1` (final, after the
  queue/notify gates) →
  `test result: ok. 1030 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.72s`.
- `cargo check -p zeron --tests` → `Finished dev profile` (the app crate still
  builds).

## Files changed

- `crates/ui/src/composer.rs` — `chat_override` + `Composer::for_chat` +
  `bound_chat_id`/`bound_chat_row` (every `selected_chat` read now routes
  through them); `/sidechat` app-level slash entry deduped like `/compact`;
  `send()` intercept before the busy branch emitting
  `ComposerEvent::OpenSidechat`; nesting refusal; RunRequest config taken from
  the bound chat's row; footer reopen pill; `send_prompt`; tests.
- `crates/ui/src/shell.rs` — `RightSurface::Sidechat(u64)`, `SidechatTab`,
  `open_sidechat` / `add_sidechat_surface` / `send_into_sidechat`,
  picker + plus-menu rows, tab render with the context note, close teardown,
  `ComposerEvent::OpenSidechat` handler; tests.
- `crates/ui/src/shell/spaces.rs` — `nest_sidechats` used by
  `render_active_rows` (↳ inset) and `grouped_visible_order` (jump order);
  test.
- `crates/ui/src/queue.rs` — the queue panel and the Cmd+Enter
  latest-queued shortcut are no-ops for a bound composer.
- `crates/ui/src/state.rs` — `open_sidechat_tabs: HashSet<String>`,
  `latest_sidechat_of`.

## Assumptions not verified

- No engine was running, so the `CreateSidechat` RPC path (params shape
  `{sourceChatId, chatId, targetDeviceId}`, reply decoded as `Chat`) is
  verified only against `crates/engine/src/rpc.rs` + `workspace_host.rs`, not
  live.
- The tab's composer sends through the ordinary `QUEUE_COMMAND` Run path with
  the sidechat's own row config; that shape is exercised by the existing send
  code, not by a new unit test (a unit test cannot reach it — `send()` returns
  early without an engine handle, and no `crates/ui` test injects one). The
  shell test proves the hand-off up to that early return (the tab composer
  fails with "Engine not connected" on the SIDECHAT's id).
- "Tab is not showing" is read narrowly: the pill hides while the sidechat's
  tab exists in the strip (`open_sidechat_tabs`), even if the pane is
  collapsed.

## Residual risks

- The `CreateSidechat` reply handler reads `panel_key`/`active_chat` at reply
  time: navigating to another chat during the RPC lands the new tab on that
  chat's strip.
- The tab's transcript is a doc-override instance, which by construction shows
  no optimistic echoes (`transcript.rs:4028`): a sidechat prompt appears when
  the host writes it back, not instantly.
- The context meter in the tab's footer still reads `state.context_usage` (the
  selected chat's) — cosmetic, one number.
- The tab composer never receives `set_available_width`, so responsive controls
  fall back to `COMPOSER_MAX_WIDTH`.
- `send()`'s new-chat failure cleanup still reads `state.selected_chat`
  directly; it is unreachable for a bound composer (`is_new` is always false
  there).

## Incidental findings

- `set_compact_failure` was renamed `set_chat_failure` (same body, four call
  sites) because the sidechat path shares it.
- `compact_focus` and `sidechat_prompt` now share `app_command_argument`.
- `rustfmt --edition 2024 crates/ui/src/shell.rs` reformats
  `crates/ui/src/shell/tabs.rs` as collateral; that file was restored to HEAD.
