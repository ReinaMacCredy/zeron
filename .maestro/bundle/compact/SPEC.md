# SPEC — compact

## Problem

Manual context compaction only half-works and only on Claude. The Claude CLI advertises `compact` (95 commands on 2.1.273, probed 2026-09-17), zeron passes the text through, the CLI compacts, but the resulting `system`/`compact_boundary` frame is swallowed by the catch-all in `crates/harness/src/claude/normalize.rs` (only `init`, `task_started`, `task_notification` are handled) and the context meter is never reset because `contextUsage` is cleared only for runs without `resume` (`crates/engine/src/sessions.rs:1437-1439`). Codex never receives `thread/compact/start`; its `thread/compacted` notification is routed but unused (`crates/harness/src/codex/normalize.rs:696-700`). OpenCode's `session.summarize` is never called. Cursor exposes nothing. t3code solved the same problem across five providers with a per-provider compaction capability (`apps/server/src/provider/Services/ProviderAdapter.ts:35-43`, `ProviderService.ts:1702-1830`).

## Solution

Per decision d6 (t3code shape):

- `Harness` gains `fn compaction(&self) -> Compaction` with `Compaction::{ Native, SlashText(String), None }` (default `None`), and `async fn compact(&self, request: RunRequest, controls: RunControls) -> Result<BoxStream<AgentEvent>>` implemented only by `Native` drivers. Per harness: Claude `SlashText("/compact")`; Codex `Native` (`thread/resume` then `thread/compact/start`, resolve on `thread/compacted`); OpenCode `Native` (`session.summarize`; whether the endpoint is synchronous is unverified, see VERIFY 9); ACP `SlashText("/compact")` only when the agent's advertised commands include `compact`, else `None`; Cursor `None` in this slice (`/compress` through the shim is unverified).
- New `AgentEvent::Compacted { pre_tokens: Option<u64>, post_tokens: Option<u64> }` (`crates/proto/src/agent.rs`). Claude maps `compact_boundary.compact_metadata.pre_tokens`; Codex maps `thread/compacted` plus the next `thread/tokenUsage/updated`; OpenCode maps summarize completion.
- Engine `compact(chat_id, focus: Option<String>)`: refuse when a run is live on the chat or a compaction is pending (one pending compaction per chat); `SlashText` runs the harness with prompt = command text (+ focus for Claude) through the normal run path but with the user entry suppressed and only `Compacted`, `Error`, `SessionStarted`, `ContextUsage` folded; `Native` calls `Harness::compact`. Wait for `Compacted` with a timeout (2 minutes, routine); on timeout mark the chat so a retry is refused until the next successful turn (t3code `timedOutNativeCompactions`). A run that ends without `Compacted` (Claude "Not enough messages to compact.") surfaces the result text as a notice, writes nothing. Messages held in the queue (`holdForTurnEnd`) stay held through a compaction run and release into the next real turn, never into the compaction run itself.
- On `Compacted` (decision d5): host pushes one `MessageRole::System` entry (`crates/doc/src/schema.rs:34-40`, `push_message` at `:357-369`) with text `Context compacted` and, when present, `pre_tokens → post_tokens`; then `clear_context_usage` (`schema.rs:344-348`) and, if `post_tokens` is known, set it. History above the entry is untouched.
- Composer (decision d8): `send()` intercepts a prompt whose first token is `/compact` (`crates/ui/src/composer.rs:5952`; tokenizer `slash_token` at `:3913`), passes the remainder as focus, and calls the new compact RPC; no `Run` is built and no user entry appears. `/compact` is an app-level slash entry on every harness, deduped by name against harness-advertised lists. The context meter hover card (`crates/ui/src/context_usage.rs:85-108`) gains a `Compact context` button. Both are disabled with a reason while a run is live, while a compaction is pending, or when the capability is `None` ("<harness> does not support compaction"). While pending the composer shows `Compacting…`.
- Transcript renders `MessageRole::System` entries as a centered divider row (desktop `crates/ui/src/transcript.rs`; edge `edge/src/session-doc/render-parts.ts`; iOS `apps/ios/Zeron/Transcript/TranscriptRows.swift:118` currently draws non-user entries as assistant rows).

## Scope

- `crates/proto`: `AgentEvent::Compacted`, `Compaction` enum.
- `crates/harness`: trait additions; claude (`compact_boundary` parse), codex (`thread/compact/start`, `thread/compacted`), opencode (`session.summarize`), acp (capability from advertised commands), cursor (`None`).
- `crates/engine`: `compact` method + RPC verb, pending/timeout bookkeeping, System entry + meter reset.
- `crates/doc`: fold `Compacted`; System entry writer.
- `crates/ui`: composer interception + app-level slash entry, meter button, `Compacting…` state, System row renderer.
- `edge/`: System row renderer in `render-parts.ts` (role already typed at `messages.ts:12`).
- `apps/ios`: System row renderer (slice 2; until then the entry shows as an assistant-styled row, which is tolerable, not a crash).
- Out of scope: auto-compaction threshold settings (t3code `autoCompactWindow`), journal compaction (`crates/engine/src/run_journal.rs:10`), Cursor `/compress`.

## Anti-goals

Each anti-goal gets a matching VERIFY.md check.

1. Compaction never deletes, rewrites or reorders existing doc entries; it only appends one System entry.
2. Zeron never runs its own summarization model call; the only model work is the harness's own compaction.
3. No user entry for `/compact` is ever written.
4. Never two compactions in flight on one chat.
5. Compaction never changes `harness_session_cwd` and only updates `harness_session_id` to an id the harness itself reported for that run.
6. No new settings key and no auto-compaction trigger.
7. OpenCode's driver-side `/command` routing (`crates/harness/src/opencode/mod.rs:1901-1962`) keeps working for other advertised commands.

## Decisions

d5, d6, d8

## Derived artifacts

`.maestro/recon-sidechat-compact.md` section 2 (per-harness `ContextUsage` sources and resume mechanics) was produced by a read-only Explore agent with `rg` over `crates/`; it under-approximates runtime behaviour (no live probe of Codex or OpenCode compaction), which VERIFY 8 and 9 cover.

## Red tests

- claude normalize: a `system`/`compact_boundary` frame with `compact_metadata.pre_tokens` yields `AgentEvent::Compacted { pre_tokens: Some(n), .. }`; frames without it still yield nothing else new.
- codex driver (fake-codex fixture): `compact` issues `thread/compact/start` on the resumed thread id and completes on `thread/compacted`; times out cleanly when the fixture never answers.
- opencode driver (fake server): `compact` hits the summarize endpoint for the session id and yields `Compacted`.
- engine: `compact` on an idle chat appends exactly one System entry, leaves earlier entries byte-identical, clears then sets `contextUsage`; a second concurrent `compact` is refused; `compact` while a run is live is refused; a run ending without `Compacted` writes no entry.
- doc: `Compacted` folds to a System entry, not an assistant part.
- ui (`composer.rs` tests): `/compact` and `/compact focus on auth` are intercepted in `send()` with the right focus and build no `Run`; the slash popup lists `/compact` once on a harness that also advertises it; button/entry disabled reason for a `None` harness and for a live run.
- edge (`render-parts` tests): a `system` entry renders as a divider and never as an assistant bubble.
