# Handback — w9 sidechat engine

Written by the coordinator: the implementing agent was cut off by an API rate limit after its tests passed and before it could write this packet. Content reconstructed from the diff and the test run.

## Status

DONE

## Claim

`Chat.sidechat_source_chat_id` persists as `sidechatSourceChatId` through both registry encode sites and `RawChat`. `WorkspaceHost::create_sidechat` creates the sidechat in the source's space with the source's cwd, branch, checkout and `ChatConfig`, title `Sidechat: <source title>`, no harness session, and refuses a source that is itself a sidechat. `Inner::sidechat_harness_prompt` wraps the prompt at all three hand-offs (cold dispatch in `drive_run` after the retry snapshot, the warm steering mailbox, and the steer path that held-queue release uses) with the astra boundary and, until the sidechat has a completed assistant entry, a 48k-char newest-first copy of the source's user/assistant text parts; the doc entry keeps the typed text. RPC `CreateSidechat { sourceChatId, chatId }` returns the Chat row.

## Proof

```
$ cargo test -p zeron-engine --test sidechat --test compact
test an_empty_source_yields_no_context_block ... ok
test creation_inherits_the_source_and_refuses_nesting ... ok
test a_sidechat_turn_leaves_the_source_untouched ... ok
test the_copy_drops_tool_parts_and_respects_the_budget ... ok
test a_steered_prompt_is_wrapped_too ... ok
test the_first_turn_carries_the_source_copy_and_later_turns_do_not ... ok
test result: ok. 6 passed (sidechat); 6 passed (compact)
$ cargo test -p zeron-doc            # 107 passed
$ cargo check --workspace --tests    # clean, pre-existing warnings only
```

## Files changed

crates/proto/src/entities.rs, crates/doc/src/{registry.rs,registry/tests.rs,workspace.rs}, crates/engine/src/{rpc.rs,sessions.rs,workspace_host.rs}, crates/engine/tests/{sidechat.rs (new),workspace_sync.rs}, crates/rpc/src/lib.rs, crates/sync/tests/{registry_client.rs,registry_edge.rs}, plus `sidechat_source_chat_id: None` in five crates/ui test literals.

## Assumptions not verified

- Title generation on a sidechat's first prompt: the drive_run wrap sits after the titler's capture, so the titler sees typed text; whether it still runs for a chat that already has a title was not checked here.
- `room_gen: Some(2)` on the created row mirrors what the UI-created chats get; not verified against `create_chat` at the time of writing.

## Residual risks

- The copy reads the source doc through `doc_host.open`, which is a host-side read; a sidechat whose source lives on another device gets an empty copy.
