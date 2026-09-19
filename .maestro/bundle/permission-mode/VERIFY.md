# VERIFY — permission-mode

Results hold the latest run only (date + commit). Run 2026-09-17 on the
worktree at feat/permission-mode (uncommitted, base 4732b13f).

| # | Scenario | Check | Result |
|---|---|---|---|
| 1 | Old-wire compat | `cargo test -p zeron-proto run_request_permission_mode_defaults_to_full` | PASS (proto 107/107) |
| 2 | Row copy | engine builds `RunRequest.permission_mode` from `ChatConfig` (sessions.rs, doc_host.rs `request_from_chat_row`) — `cargo test -p zeron-engine --lib` | PASS (engine lib 181/181, doc 27/27) |
| 3 | Routing key | `RuntimeConfig` includes the mode, so a switch replaces the runtime and resumes | PASS by construction (field added to the compared struct; `can_route` test still green) |
| 4 | Chip renders | fresh Zeron Dev 7 (worktree target/macos-dev7, port 49783) screenshot: chip between sidechat pill and ring; Full in warning tone | PASS — /tmp/pm-2s.png (chip), /tmp/pm-6s.png (dropdown via `ZERON_OPEN_PICKER=permission`) |
| 5 | Chip follows the row | `Mutate setChatConfig permissionMode ask|auto` over RPC → chip reads "Ask for approval" / "Approve for me", open dropdown's check moves | PASS — /tmp/pm-ask-foot.png, /tmp/pm-auto-foot.png, /tmp/disp1bs.png |
| 5b | Pick writes the row | click a dropdown row → `setChatConfig` | NOT RUN LIVE — a Little Snitch modal for a peer's Dev 6 opencode process blocks every CGEvent click; the pick uses the same `update_chat_config` path the model picker uses |
| 6 | Full unchanged | Full path: Claude `bypassPermissions` + auto-allow; Codex `never` + `danger-full-access`; OpenCode auto-yes; ACP bypass — `cargo test -p zeron-harness --lib` | PASS (158 passed; the 1 failure is the known base failure, row 15) |
| 7 | Claude Ask | Dev 7 chat `chat-claude-ask` (harness claude-code, mode ask) spawned `claude … --permission-mode default` without `--dangerously-skip-permissions` (ps, pid 97764); `permission_mode_selects_the_cli_flags` pins Ask/Full → `default`, Auto → `acceptEdits` | WIRE PASS; the turn itself never progressed (no assistant part in WatchDocMessages after 80s) — blocked behind the same Little Snitch modal; interrupted |
| 8 | Claude Auto | CLI probe 2.1.274 with stdin held open: `acceptEdits` auto-allows Write and read-only Bash, no `can_use_tool`; `default` raises `can_use_tool` for Write; `--permission-mode auto` is downgraded to `default` in print mode (init frame reports `default`, Write asks) | PASS (probes /tmp/probe-ae3, /tmp/probe-ae4, /tmp/probe-auto) — Auto maps to `acceptEdits` |
| 9 | Codex Ask/Auto | `approval_policy` + `sandbox` mapping (`cargo test -p zeron-harness --lib codex::catalog`); run-path `thread/resume` reuses `start_params` so a switch carries the policy | PASS (unit); live turn NOT RUN (Little Snitch) |
| 10 | Codex worktree | `codex exec -s workspace-write -c approval_policy="never"` from the linked worktree on `feat/permission-mode` wrote probe-ws.txt on 0.154.0 | PASS |
| 11 | OpenCode | `auto_mode_allows_workspace_edits_and_asks_for_the_rest`, `permissions_without_auto_approve_require_an_explicit_answer`, `permissions_stay_session_scoped_and_never_persist_grants` | PASS |
| 12 | ACP | `claude_acp_mode_select_follows_the_permission_mode`, codex-acp mode test extended (Auto = agent default, Ask = read-only) | PASS |
| 13 | Titles / compaction | titles test asserts read-only + `permission_mode == Ask` (same CLI flags as before); compaction still forces full access | PASS after pinning titles to Ask |
| 14 | UI suite | `cargo test -p zeron-ui --lib -- --test-threads=1` | PASS (1037/1037) |
| 15 | Base failures | `shell_env::unix::tests::falls_back_when_interactive_attempt_hangs` (harness lib) and `e2e::generated_image_is_materialized_before_publication_and_survives_reopen` (uploads-dir `/var` symlink prefix) fail here; both recorded as failing at base on this Mac on 2026-09-17 (memory zeron-fork-fmt-and-test-gotchas); shell_env.rs and the uploads path are untouched by this diff | KNOWN BASE FAILURES |
| 16 | Engine integration suites | `cargo test -p zeron-engine --tests --no-fail-fast` with TMPDIR resolved through /private | PASS for every suite except `local_first::online_runtime_shutdown_stops_edge_workers_and_retires_the_graph` ("edge received requests after shutdown returned: 8 vs 5"), the documented environmental failure from running Zeron instances' preview discovery probing a test process whose cwd is under the checkout; reproduces with Dev 7 quit while peers' Dev 5/6 and production Zeron run |
