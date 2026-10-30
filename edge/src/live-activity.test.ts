import { describe, expect, it } from "vitest";
import { contentFor, finalState, liveActivityPayload, liveRows, newlyAsking, parseTarget, stepOf } from "./live-activity";
import type { Row } from "./registry-core";

const NOW = 1_800_000_000_000;

const row = (kind: string, id: string, fields: Row["fields"], deleted = false): Row => ({ kind, id, seq: 1, deleted, fields, clocks: {} });

const session = (id: string, status: string, startedAt?: number, updatedAt = NOW) =>
  row("sessions", id, { chatId: id, status, updatedAt, ...(startedAt === undefined ? {} : { startedAt }) });

const chats = new Map<string, Row>([
  ["a", row("chats", "a", { title: "Fix sync", spaceId: "s1", deviceId: "mac", branch: "main", lastMessagePreview: "\n  Running tests  \nmore" })],
  ["b", row("chats", "b", { title: "  ", spaceId: "s2", deviceId: "mac" })],
  ["c", row("chats", "c", { title: "Side", parentChatId: "a" })],
  ["d", row("chats", "d", { title: "Old", archived: true })],
  ["e", row("chats", "e", { title: "Release", deviceId: "mac" })]
]);
const spaces = new Map<string, Row>([
  ["s1", row("spaces", "s1", { name: "zeron" })],
  ["s2", row("spaces", "s2", { path: "/Users/r/Code/site/" })]
]);
const devices = new Map<string, Row>([["mac", row("devices", "mac", { name: "MacBook" })]]);

const build = (sessions: Row[]) =>
  liveRows(sessions, (id) => chats.get(id), (id) => spaces.get(id), (id) => devices.get(id), NOW);

describe("liveRows", () => {
  it("keeps live top-level chats, asks first, then oldest first", () => {
    const rows = build([
      session("a", "working", NOW - 60_000),
      session("b", "awaitingInput", NOW - 10_000),
      session("c", "working"),
      session("d", "working"),
      session("e", "working", NOW - 120_000)
    ]);
    expect(rows.map((r) => [r.id, r.state])).toEqual([
      ["b", "asking"],
      ["e", "working"],
      ["a", "working"]
    ]);
    expect(rows[2]).toEqual({
      id: "a",
      title: "Fix sync",
      project: "zeron",
      branch: "main",
      state: "working",
      since: (NOW - 60_000) / 1000,
      step: "Running tests"
    });
    expect(rows[0]?.title).toBe("New session");
    expect(rows[0]?.project).toBe("site");
    expect(rows[1]?.project).toBe("MacBook");
  });

  it("drops idle, errored and stale sessions", () => {
    expect(build([session("a", "idle"), session("b", "errored"), session("e", "working", NOW, NOW - 60_000)])).toEqual([]);
  });
});

describe("contentFor", () => {
  const rows = build([session("a", "working", NOW - 60_000), session("b", "awaitingInput", NOW), session("e", "working", NOW)]);

  it("groups every live session with totals", () => {
    const state = contentFor({ which: "all", pinned: [] }, rows);
    expect(state?.rows.map((r) => r.id)).toEqual(["b", "a", "e"]);
    expect([state?.running, state?.asking]).toEqual([2, 1]);
  });

  it("filters pinned and asks", () => {
    expect(contentFor({ which: "pinned", pinned: ["e"] }, rows)?.rows.map((r) => r.id)).toEqual(["e"]);
    expect(contentFor({ which: "asking", pinned: [] }, rows)).toEqual({ rows: [rows[0]], running: 0, asking: 1 });
    expect(contentFor({ which: "pinned", pinned: [] }, rows)).toBeNull();
  });

  it("follows one chat per activity and ends when it stops", () => {
    expect(contentFor({ chatId: "a", which: "all", pinned: [] }, rows)).toEqual({ rows: [rows[1]], running: 1, asking: 0 });
    expect(contentFor({ chatId: "zzz", which: "all", pinned: [] }, rows)).toBeNull();
  });

  it("keeps at most three rows", () => {
    const many = build(["a", "b", "e"].map((id) => session(id, "working", NOW)).concat(session("x", "working")));
    expect(contentFor({ which: "all", pinned: [] }, [...many, ...many])?.rows).toHaveLength(3);
  });
});

describe("payloads", () => {
  it("alerts only on a new ask", () => {
    const before = contentFor({ which: "all", pinned: [] }, build([session("a", "working", NOW)]));
    const after = contentFor({ which: "all", pinned: [] }, build([session("a", "awaitingInput", NOW)]));
    expect(after).not.toBeNull();
    if (after === null) return;
    expect(newlyAsking(before, after).map((r) => r.id)).toEqual(["a"]);
    expect(newlyAsking(after, after)).toEqual([]);
  });

  it("ends on the last rows marked done, with a dismissal date", () => {
    const last = contentFor({ which: "all", pinned: [] }, build([session("a", "working", NOW)]));
    const payload = liveActivityPayload("end", finalState(last), 100, { dismissalDate: 1000 });
    expect(payload.aps.event).toBe("end");
    expect(payload.aps["dismissal-date"]).toBe(1000);
    expect(payload.aps["content-state"].rows[0]).toMatchObject({ id: "a", state: "done" });
    expect(payload.aps["content-state"].rows[0]).not.toHaveProperty("since");
  });

  it("an update with an alert carries a sound", () => {
    const payload = liveActivityPayload("update", { rows: [], running: 0, asking: 0 }, 5, { alert: { title: "Needs you", body: "Fix" } });
    expect(payload.aps).toMatchObject({ timestamp: 5, event: "update", alert: { title: "Needs you", body: "Fix", sound: "default" } });
  });
});

describe("parseTarget and stepOf", () => {
  it("defaults and clamps what the phone sends", () => {
    expect(parseTarget("id", "dev", { which: "nope", endAfter: 1e9, pinned: ["a", 3] })).toEqual({
      id: "id",
      device: "dev",
      which: "all",
      alert: true,
      endAfter: 4 * 60 * 60,
      pinned: ["a"]
    });
    expect(parseTarget("id", "dev", { chatId: "c", which: "asking", alert: false, endAfter: 0 })).toMatchObject({ chatId: "c", which: "asking", alert: false, endAfter: 0 });
  });

  it("takes the first line, at most 80 characters", () => {
    expect(stepOf(undefined)).toBeUndefined();
    expect(stepOf("   \n")).toBeUndefined();
    expect(stepOf("x".repeat(100))).toHaveLength(80);
  });
});
