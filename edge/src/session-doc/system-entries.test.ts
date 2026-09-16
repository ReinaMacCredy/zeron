import { describe, expect, it } from "vitest";
import { fromDocParts, toDocParts, splitMessageEntry, joinContinuations } from "./messages";
import { toRenderParts } from "./render-parts";

// The compaction boundary is a host-written `system` entry. The edge has no
// per-role renderer — it materializes the tail and the clients draw it — so
// what it owes the divider row is that `role` and the text survive the
// doc transforms untouched, never collapsing into an assistant entry.
describe("system entries", () => {
  const boundary = { kind: "text" as const, id: "t0", text: "Context compacted · 22.8k → 2.3k tokens" };
  const entry = { id: "b1", role: "system" as const, deviceId: "host", createdAt: 1, parts: toDocParts([boundary]), status: "complete" as const };
  it("keeps its role and text through the doc part transforms", () => {
    expect(fromDocParts(toDocParts(toRenderParts([boundary])))).toEqual([boundary]);
    expect(entry.role).toBe("system");
  });
  it("keeps its role through split/join", () => {
    expect(joinContinuations(splitMessageEntry(entry, 160))).toEqual([entry]);
  });
});
