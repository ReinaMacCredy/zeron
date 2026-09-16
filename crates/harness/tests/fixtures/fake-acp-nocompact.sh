#!/bin/sh
# Fake ACP agent that advertises commands WITHOUT `compact` — the other side
# of the compaction capability check. Handshake only; no session is ever run.

emit() { printf '%s\n' "$1"; }
rid() { printf '%s' "$1" | sed 's/.*"id":\([0-9]*\).*/\1/'; }
has() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

read -r line || exit 1
has "$line" '"method":"initialize"' || exit 1
emit "{\"id\":$(rid "$line"),\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"_meta\":{\"availableCommands\":[{\"name\":\"goal\",\"description\":\"Set a goal\"}]}}}}"
exec sleep 30
