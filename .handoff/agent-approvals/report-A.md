# Report A: U1 + U2 + U3 (engine core)

```text
Status: DONE
Claim: U1-U3 work as SPEC describes. A lead can raise its child's access mid-run with set_thread `access`; the raise reaches the live run and answers the pending approvals the new mode covers. respond_to_input `scope: "run"` answers Yes and lets the rest of that run's same-kind approvals through without asking, leaving the chat config unchanged.
Proof: the commands and results below, including two mutation checks. Layers: unit (harness approval, engine agent_gate), integration against a mock harness (engine tests/agent_approvals.rs plus existing suites), compile (--all-targets on eight crates).
Assumptions not verified: the set_thread `access` reply note (raised / lowered / capped) has no test; agent_context_budget being green at HEAD is assumed.
Residual risks: see "Residual risks".
Incidental findings: see the end.
```

The live-raise tests are falsifiable: with `raise_allowance` forced to return false, the three live tests fail; with the code restored, all pass. One test needed a fix to its wait: the doc applies `InputResolved` asynchronously, so it now waits for the card count instead of asserting it at once.

Nothing is committed or pushed. The `apps/zgx/**` changes in the tree belong to the other agent; I didn't touch them. No comment lines were added.

## Design choices

- **Typed action** (U1): `zeron_proto::ApprovalAction { Edit, Command, Tool }`, added as `UserInputQuestion.action: Option<_>` (serde default, skipped when absent). `approval::command`, `edit` and `tool` set it. Plan approval and every direct `approval::question` site (Codex permissions and UI drive, OpenCode external directory, Muse staged command) stay untyped, so they are never auto-answered and refuse `run`. No driver logic changed.
- **One coverage rule**, in `approval.rs`:
  - `covered(access, questions)` reuses `allowed_without_asking`. A request counts only if it's non-empty and every question is a typed approval with no `kind` (so consent, foreground and secret questions never count).
  - `least_covering` returns the first mode in `AccessMode::ALL` that covers the request.
  - `allow_all` builds a Yes for every question id, which passes `approval::approved`.
- **Live allowance** (U1): `RunHandle.allowance: Arc<Mutex<AccessMode>>`, starting at the request's access. `PendingInput` now keeps its questions.
  - The `request_input` closure checks the allowance while holding the pending lock. A covered request is answered Yes at once and never shown.
  - `raise_allowance` takes the pending lock, then the allowance, so lock order is the same everywhere. It only raises, then answers every covered pending request (skipping secrets), sends the Yes to the harness and emits `InputResolved`.
- **`SessionsEngine::raise_run_access`** (U2): raises the allowance and lifts `runtime_config.request.access` and `last_requests[chat].access` when the new mode is higher, so the next dispatch reuses the process. As you accepted, this also ends computer-consent prompts for that run.
- **Where the raise fires**: `provider_switch::apply` calls it after the config write, **only when the new access is above the previous stored config's access**. The review caught that an unconditional call would let a model-only config change lift a run launched below its config and auto-approve its pending requests. The test `a_config_write_that_does_not_raise_access_answers_nothing` covers this; it fails under the old behavior and passes now.
- **U2 gate** (`agent_gate.rs`, setChatConfig):
  - An agent's own chat goes through the existing path unchanged.
  - Another chat is allowed only if the stored record shows ownership: `child_kind` set and `created_by_chat_id == caller`, or `delegation.delegated_by_chat_id == caller`.
  - The config is rebuilt from the stored record with only `access` replaced, capped by the access half of `cap_access` (the sandbox is untouched). `note` is dropped, and a child with no stored config is refused.
- **U2 MCP**: `set_thread` takes `access` (enum schema) for one `threadId`.
  - It sends the target's stored config with `access` replaced, then reads it back.
  - The reply has `access` plus a note: "raised now: …", "lowered from its next run; …", "unchanged", or "; capped at this chat's own access".
  - No new tool switch (your decision 4); it rides `set_thread`.
- **U3**: `QueueCommandParams.scope: RespondScope { Once (default), Run }` in rpc.rs; it is never written to the doc.
  - Before queueing, `run_scope_access` checks that the run is live on this engine ("can only be answered once" otherwise), that the request is waiting, that `least_covering` exists (else "only a command, file change or tool approval can be approved for the run; answer this one once"), and that every question is answered Yes.
  - After queueing, `allow_for_run` raises only the allowance and skips the request the queued answer resolves.
  - The gate now refuses `scope: "run"` from a non-Full agent whatever the labels say, which closes the "No"-label bypass.
  - The MCP adds `"scope":"run"` only for run, so the default wire shape is unchanged.

## Files changed

- **Logic**:
  - `crates/proto/src/agent.rs`
  - `crates/harness/src/approval.rs` (+ unit test)
  - `crates/engine/src/sessions.rs`
  - `crates/engine/src/provider_switch.rs`
  - `crates/engine/src/rpc.rs`
  - `crates/engine/src/rpc/agent_gate.rs` (+ 2 tests)
  - `crates/mcp/src/lib.rs`
- **New test file**: `crates/engine/tests/agent_approvals.rs`, 6 tests, mock harness only.
- **One-line `action: None,` edits** (your decision 1, plus sites inside the owned crates):
  - client: `demo/missions.rs`, `demo/transcripts.rs` (5), `session/tests.rs`
  - doc: `schema.rs`
  - engine: `computer/engine_seams.rs` (2), `computer/visible_use.rs`, `doc_host_delegation.rs`, `secret_requests.rs`; tests `computer_run.rs` (4), `e2e.rs` (5), `message_queue.rs`, `side_chats.rs`, `supervisor_stalls.rs`, `thread_tools.rs`
  - harness: `acp/mod.rs`, `claude/mod.rs`, `codex/elicitation.rs` (2), `codex/mod.rs`, `mock.rs` (2), `muse/session.rs` (2), `opencode/mod.rs`, `pi/ui.rs`
  - mobile: `layout/tests.rs` (3)
  - proto: `computer/cards.rs`, `view.rs`
  - ui: `examples/question-answers-fixture.rs`, `composer.rs`, `composer/ask_pill.rs`, `composer_dictation_tests.rs`, `transcript.rs` (5)
- **Formatting**: `approval.rs`, `provider_switch.rs` and `agent_approvals.rs` went through `rustfmt` via stdin. sessions.rs, rpc.rs, agent_gate.rs, proto `agent.rs` and mcp `lib.rs` were not rustfmt-clean at HEAD, so I formatted only my own hunks by hand. agent_gate.rs keeps its wide local style.

## Commands and results

Before each build: the env vars were unset and `scripts/gc-target.sh` reported "no crate over 8G".

| Command | Result |
|---|---|
| `cargo check --all-targets -p zeron-proto -p zeron-harness -p zeron-engine -p zeron-mcp -p zeron-client -p zeron-doc` | exit 0; warnings only in files I didn't touch |
| `cargo check --all-targets -p zeron-ui -p zeron-mobile` | Finished, exit 0 |
| `cargo test -p zeron-harness --lib approval` | 8 passed, incl. `a_mode_covers_only_typed_approvals_it_allows` |
| `cargo test -p zeron-engine --lib agent_gate` | 9 passed; `an_agent_cannot_raise_a_chats_config` unedited and green |
| `cargo test -p zeron-engine --test agent_approvals` | 6 passed; earlier 5-test version passed 3 runs in a row |
| `cargo test -p zeron-mcp --lib` | 122 passed |
| `cargo test -p zeron-engine --test provider_switch` | 11 passed |
| `cargo test -p zeron-engine --test computer_run` | 29 passed |
| `cargo test -p zeron-engine --test message_queue` | 36 passed |
| `cargo test -p zeron-engine --test thread_tools -- approval input respond` | 4 passed |
| `cargo test -p zeron-engine --test e2e -- input approval question` | 5 passed |
| `cargo test -p zeron-engine --test agent_context_budget` | fails as expected: `toolListJsonBytes` 56208 → 56741 (+533 B). Left for U5 to re-measure; JSON untouched |

The two new gate tests are `an_agent_changes_only_the_access_of_a_thread_it_owns_capped_at_its_own` and `approving_for_the_run_needs_a_full_access_agent_even_with_a_no`.

Brief evidence, test by test:

| Required behavior | Test |
|---|---|
| A child's pending command is answered when the lead raises it to Full | `a_raise_to_full_answers_the_childs_pending_command` |
| A raise to Auto-accept edits answers a pending edit but not a pending command | `a_raise_to_auto_accept_edits_answers_the_edit_and_leaves_the_command` |
| A non-owner chat is refused | gate test above: a stranger chat, and a linked chat without `child_kind` |
| A raise above the caller is capped | `a_raise_is_capped_at_the_leads_own_access`, plus the gate test, which also shows harness, model, sandbox and disabled tools are kept |
| `scope: "run"` lets the next same-kind approval through, chat config unchanged | `scope_run_lets_the_next_command_through_and_keeps_the_chat_config`, which also checks that a "No" with run is refused |
| Plan approval refuses run | `scope_run_refuses_a_plan_approval` |

Two mutation checks:
- `raise_allowance` forced to return false: 3 live tests fail.
- The `provider_switch` guard replaced with `if true`: the no-raise test fails.

Both were restored and re-verified.

## Residual risks

1. **How long `scope: "run"` lasts.** The allowance lives on the `RunHandle`. A steerable runtime keeps its handle across turns, so "run" also covers later turns, including ones the person starts, until the process is replaced (by a config change of access, a harness switch, or reaping). My test harness isn't steerable, so no test shows this.
2. **Computer consent after U2.** A Supervised run raised by U2 stops asking for computer consent, because `computer_caller` reads the launch access. You accepted this.
3. **Muse at Auto-accept edits.** Muse asks at any mode below Full. If a Muse chat is set to Auto-accept edits (through `set_thread` access; the pickers don't offer it), its edit approvals are now answered by the allowance. The other typed-approval drivers (Claude, Codex, OpenCode, ACP, mock) gate on their access before asking, so a run that was never raised has no newly silent approvals.
4. **Untyped approvals.** Codex permissions and UI drive, OpenCode external-directory access, the Muse staged command, and ACP's direct-built question still go to the person or lead after a raise.
5. **Tool switch.** With all three of `set_thread`'s action switches off, the tool is hidden, but a direct `access` call isn't refused by a switch of its own (decision 4).
6. **`scope` stays local.** It doesn't travel to chats hosted elsewhere; those get the plain refusal.
7. **Unknown action values.** `ApprovalAction` has no lenient deserializer. A future variant would fail to parse in this build; older builds simply ignore the field.

## Incidental findings

- `cargo check` warnings in files I didn't touch: unused import `TOOL_GROUPS` in `crates/client/src/host_settings/profile_tools.rs`, an unused doc comment at sessions.rs:7926, an unused import in `chat2_host.rs:625`, and `predictions.rs:143` "does not need to be mutable".