# VERIFY — sidechat

Scenarios point at work acceptance/claims; do not restate them.

| # | Scenario | Work / acceptance | Result |
|---|----------|-------------------|--------|
| 1 | On a Claude chat with three turns, type `/sidechat which file did you change?`; a sidechat opens in the right pane and answers from the copied context | w4 acceptance: side conversation sees source context | |
| 2 | Repeat scenario 1 on Codex, OpenCode, an ACP agent and Cursor | w4 acceptance: all five harnesses | |
| 3 | Dump the source doc (`WATCH_DOC_MESSAGES` count) before and after the sidechat turn: identical | anti-goal 1 | |
| 4 | Read the source `Chat` row before/after: `harness_session_id`, `harness_session_cwd`, `contextUsage` meta unchanged | anti-goal 2 | |
| 5 | Run with a fake harness fixture that records prompts: no prompt starts with `/sidechat`; the sidechat's first prompt starts with `<sidechat_boundary>` | anti-goal 3, red test 1 | |
| 6 | `rg -n 'fork-session|thread/fork|session\.fork' crates/harness/src` returns nothing | anti-goal 4 | |
| 7 | In the sidechat pane type `/sidechat again`: refused with a notice, no chat created | anti-goal 5, red test 3 | |
| 8 | `git diff --stat crates/doc/src/schema.rs edge/src/session-doc` is empty | anti-goal 6 | |
| 9 | Close the pane: sidebar still shows the sidechat nested under the source; the source composer shows the reopen pill; clicking it restores the pane | d2 | |
| 10 | Change the model in the sidechat pane; the source chat's model picker is unchanged | SPEC Solution, last bullet | |
| 11 | `cargo test -p zeron-engine -p zeron-ui` green, including the red tests listed in SPEC | Definition of Done | PASS 2026-09-17 at d355e202: engine sidechat 6/6 (`the_first_turn_carries_the_source_copy_and_later_turns_do_not`, `a_steered_prompt_is_wrapped_too`, `the_copy_drops_tool_parts_and_respects_the_budget`, `an_empty_source_yields_no_context_block`, `creation_inherits_the_source_and_refuses_nesting`, `a_sidechat_turn_leaves_the_source_untouched`), doc registry round-trip, ui lib 1035 incl. 7 sidechat tests. Scenarios 3, 4, 5, 7, 8 covered at unit level; 1, 2, 9, 10, 12, 13, 14 remain for the owner's manual pass. Scenario 10 is currently not reachable: the tab composer hides the pickers (w4 note). |
| 13 | Open the right pane picker on a chat with no sidechat: a `Sidechat` row is listed; choosing it opens a Sidechat tab with an empty composer; choosing it again later reopens the same sidechat | SPEC Solution, entry point (a) | |
| 14 | Type bare `/sidechat` and send: the Sidechat tab opens with an empty composer and no turn starts; type `/sidechat compare` and send: the tab opens and the first turn starts with "compare" | SPEC Solution, entry point (b) | |
| 12 | iOS build unchanged; the sidechat appears in the iOS chat list as a flat chat with no crash on the unknown field | SPEC Scope, edge/iOS | |
