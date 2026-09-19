# SPEC — permission-mode

Original instruction (2026-09-17, Reina): "hiện giờ claude code có auto mode
bypass etc, codex thì có yolo etc và các harness khác như opencode đồ mà zeron
chưa có cái chỗ chọn quyền đó" → design discussion → "build". Locked in the
discussion: three modes named after the ChatGPT desktop picker, one chip for
every harness, placed beside the context ring, default stays Full ("full như
hiện tại tôi muốn đổi thì tự tôi chọn").

## Problem

Zeron runs every harness unattended (Claude `bypassPermissions` + auto-allow
of every `can_use_tool`, Codex `approvalPolicy: never` + `danger-full-access`,
OpenCode `auto_approve`, ACP picks the bypass mode). The two knobs that exist
(`ChatConfig.sandbox`, `RunRequest.auto_approve`) are never set by the UI and
the Claude/Codex adapters ignore them. There is no way to make a chat ask.

## Solution

`PermissionMode { Ask, Auto, Full }` on `ChatConfig` and `RunRequest`
(serde default Full). A chip in the composer footer's trailing cluster, right
before the context ring, with a three-row dropdown. Each harness maps the
mode onto its own approval/sandbox wire; Full is byte-for-byte today's
behavior.

| | Ask for approval | Approve for me | Full access |
|---|---|---|---|
| Claude | `--permission-mode default`, every `can_use_tool` → input bridge | `--permission-mode acceptEdits`, remaining `can_use_tool` → input bridge (the CLI downgrades `auto` to `default` in print mode, probed 2.1.274) | `--permission-mode default` + auto-allow every `can_use_tool` (today's exact flags; `bypassPermissions` was only ever reached by external scripts) |
| Codex | `untrusted` + `workspace-write` | `on-request` + `workspace-write` | `never` + `danger-full-access` (unchanged) |
| OpenCode | `auto_approve=false` | classified by permission kind, else as Ask | `auto_approve=true` (unchanged) |
| ACP | agent ask/default mode if offered; permissions → bridge | agent auto mode if offered; permissions → bridge | bypass mode + auto-accept (unchanged) |

## Scope

- w20 slice 1: proto + engine routing key + row copy + chip/popover. No harness behavior change.
- w21 slice 2: Claude. w22 slice 3: Codex. w23 slice 4: OpenCode. w24 slice 5: ACP.
- Worktree `.maestro/worktree/permission-mode`, branch `feat/permission-mode`, base 4732b13f.

## Anti-goals

- Removing or repurposing `sandbox` / `auto_approve` on the wire: peers on an older build drop a config they cannot decode (VERIFY: old-wire JSON decodes to Full).
- Changing what Full does on any harness (VERIFY: Full runs the same flags/params as base).
- Title generation and compaction following the mode (VERIFY: titles stay read-only + deny, compaction stays full).
- A mode switch touching a turn already parked on a permission question (VERIFY: the parked question resolves under the old runtime).
- Persisting "always allow" grants on OpenCode/ACP from an Ask/Auto approval (VERIFY: replies are `once` / `allow_once`).
- A draft (new-session) placement for the chip: the ring is session-only, new chats start Full. Follow-up, not this bundle.

## Decisions

Recorded in the conversation, not the store: three shared levels (not per-harness pickers), chip beside the ring (not inside the model picker, not settings-only), default Full.

## Derived artifacts

`docs/research/permission-mode/demo.html` — interactive mockup of the chip and dropdown with the per-harness wire mapping; hand-written, no derivation.

## Red tests

- proto: JSON without `permissionMode` decodes to Full (added, passes at slice 1).
- claude: `build_command` args per mode; control request Deny reply shape (slice 2).
- codex: approval policy + sandbox per mode, and `thread/resume` params carry them (slice 3).
- acp: mode-select preference per level (slice 5).
