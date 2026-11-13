# Agent approvals: raise a child's access, approve for the run, Auto mode

Reina, 2026-11-13: a lead agent can approve or refuse what its children ask,
switch a child to a looser mode (auto, full access) while it runs, on every
harness, plus an Auto mode where a model approves for her ("1 2 3"; Auto =
"Cả hai": Claude Code's own `--permission-mode auto`, a Zeron-side classifier
for the other harnesses). Ships in v0.3.2.

## Facts the design stands on (read 2026-11-13)

- Every driver decides "ask or not" Zeron-side with
  `zeron_harness::approval::allowed_without_asking(access, action)` on the
  `access` captured at launch, and asks through `RunControls::request_input`.
- The engine builds that closure once per run in `RunSetup::new`
  (`crates/engine/src/sessions.rs:~638`); pending questions live in
  `RunHandle::pending_inputs`; `respond_input` resolves one. This closure is the
  single chokepoint every harness's approval passes through.
- Approvals carry `UserInputQuestion { approval: true }` built in
  `crates/harness/src/approval.rs` (headers "Approve command", "Approve file
  change", tool, plan approval `PLAN_APPROVAL_HEADER`). Computer consent is a
  separate path and stays the person's.
- `agent_gate.rs`: an agent may `setChatConfig` only its own chat (capped);
  approving another chat's question needs the caller at Full access; the gate
  detects "approves" by the literal label "yes".
- Config changes apply from the next run: Claude/OpenCode/lib `reconfigures_in_place`
  require `live.access == next.access`, else the process is replaced.
- A run launched with no prompts (Codex `approvalPolicy: never`, ACP
  `bypassPermissions`) never asks, so lowering cannot apply live.
- `AccessMode` derives `Ord` by declaration order; `cap_access` and
  `resolve_access` rank by `AccessMode::ALL`. serde alias `auto` is already
  `AutoAcceptEdits` (old `permissionMode` name).
- One-shot cheap model calls exist: `titles::one_shot_target` +
  `run_one_shot` with `cheapest_model`.

## Units

### U1 live run allowance (engine)
- Each `RunHandle` carries a live allowance (the run's effective
  `AccessMode`), starting at the request's access.
- In the `request_input` closure: an approval question (not plan approval,
  not computer consent, not a secret) whose action the live allowance covers
  is answered "Yes" at once and never shown.
- `SessionsEngine::raise_run_access(chat, access)`: only raises (a lower value
  is a no-op here; it applies at the next run as today). It sets the live
  allowance, updates the session's stored `RunRequest.access` (so the next
  dispatch does not replace the process for an access mismatch), and answers
  every pending approval the new allowance covers, emitting `InputResolved`.
- The action kind of a question must be typed, not parsed from header text:
  add an optional action field to the approval question (serde default,
  skipped when absent, so older clients still parse) set by
  `approval::command/edit/tool`.

### U2 raise a child's access (agent tool)
- `set_thread` gains `access` for one `threadId`. The engine rewrites it into
  `setChatConfig` with the child's stored config and only `access` replaced
  (never harness, model, tools, sandbox).
- Gate: allowed for the caller's own chat (as today) or a chat whose stored
  record says the caller owns it (`child_kind` set and `created_by_chat_id ==
  caller`, or a delegation owned by the caller). Never from params. Capped at
  the caller's ceiling with `cap_access`.
- After the config write, a raise calls `raise_run_access` so the live run and
  its pending approvals follow. A lower one says in the reply that it applies
  from the child's next run.
- The person's own access picker change mid-run goes through the same
  `setChatConfig` path and so also raises live. Intended.

### U3 approve for the run
- `respond_to_input` gains `scope: "once" | "run"` (default once). Not a new
  option label: labels reach every client's card, and the gate's approve test
  matches "yes" only.
- `run` answers Yes and raises that run's live allowance (not the chat config)
  to the least mode covering the question's action (edit: AutoAcceptEdits;
  command or tool: FullAccess), capped at the caller's access. Needs the caller
  at Full like any approval. Plan approval and computer consent refuse `run`.

### U4 Auto access mode
- New `AccessMode` variant between `AutoAcceptEdits` and `FullAccess`
  (declaration order and `ALL`). Wire name `auto-review` (not `auto`: taken).
  Label "Auto", description "A model approves safe actions; asks you about
  the rest."
- Covers edits without asking, like AutoAcceptEdits; other actions go to the
  classifier.
- Claude Code: launches with `--permission-mode auto` (the CLI's own
  classifier). If the CLI reads back `default` (model without auto) or still
  asks through the permission tool, those approvals reach the engine
  classifier like any other harness.
- Other harnesses: drivers treat Auto like AutoAcceptEdits on their side
  (on-request, ask for non-edits); in the engine `request_input` chokepoint,
  an approval in an Auto run goes to the classifier first.
- Classifier: one-shot on the chat's harness with `cheapest_model` (Haiku,
  Luna class), no tools, short timeout. Input: the question text, the chat's
  folder, the chat's latest user request. Output: ALLOW or ASK only. Any error,
  timeout, unparsable reply or ASK shows the question to the person (fail
  closed). An ALLOW is recorded in the transcript as answered "by Auto", not
  silent.
- Every harness listing `AccessMode::ALL` gains Auto; one whose driver cannot
  honor it leaves it out. Pickers (zgx composer access chip, iOS if it matches
  exhaustively) show it. `cap_access` treats Auto below Full, so an Auto chat
  cannot approve another chat's question and its children cap at Auto.
- Older builds (iOS, cloud machines on upstream) do not know `auto-review`:
  check what an unknown variant does in `ChatConfig` deserialization and keep
  Auto off for cloud chats if it would break their sync.

### U5 text and docs
- MCP tool descriptions (`crates/mcp/src/lib.rs`: set_thread,
  respond_to_input, delegate_task access enum, agent_profiles access enum).
- `routing/guidance.rs` (both copies) and `doc_host_delegation.rs:228`.
- `crates/engine/tests/agent_context_budget.json` re-measured.
- What's New 0.3.2 improvement lines; docs article for access modes.

## Out of scope
- Computer-use consent: stays the person's.
- Lowering a live run's access below its launch mode.
- A "for this run" button on the person's own approval card.
