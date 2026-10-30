import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { AUTH_USER_HEADER } from "../../src/env";
import type { Op } from "../../src/registry-core";

const H = { [AUTH_USER_HEADER]: "user" };
let tick = 1;
const hlc = () => `${String(tick++).padStart(13, "0")}-000000-mac`;
const session = (status: string, updatedAt = Date.now()): Op => ({
  kind: "sessions",
  id: "c1",
  op: "upsert",
  hlc: hlc(),
  set: { chatId: "c1", deviceId: "mac", status, startedAt: 1_000, updatedAt }
});

function room() {
  return env.REGISTRY_ROOMS.get(env.REGISTRY_ROOMS.idFromName(crypto.randomUUID()));
}
async function push(stub: DurableObjectStub, ops: Op[]) {
  const r = await stub.fetch("https://registry/push?device=mac", { method: "POST", headers: H, body: JSON.stringify({ batch: crypto.randomUUID(), ops }) });
  expect(r.status).toBe(200);
}
async function activities(stub: DurableObjectStub) {
  const r = await stub.fetch("https://registry/stats", { headers: H });
  return (await r.json<{ liveActivities: Array<{ device: string; chatId: string | null; which: string; stamp: number }> }>()).liveActivities;
}
async function register(stub: DurableObjectStub, id: string, body: Record<string, unknown> = {}) {
  return stub.fetch(`https://registry/live-activity?device=ios-1&id=${id}`, {
    method: "POST",
    headers: H,
    body: JSON.stringify({ token: "ab".repeat(32), environment: "sandbox", ...body })
  });
}

describe("session Live Activity on a real RegistryRoom", () => {
  it("registers, re-registers and removes an activity", async () => {
    const stub = room();
    expect((await register(stub, "a1")).status).toBe(200);
    expect((await register(stub, "a1", { which: "asking" })).status).toBe(200);
    expect(await activities(stub)).toEqual([{ device: "ios-1", chatId: null, which: "asking", stamp: 0 }]);
    expect((await register(stub, "a2", { token: "nope" })).status).toBe(400);
    expect((await stub.fetch("https://registry/live-activity?device=ios-1", { method: "POST", headers: H, body: "{}" })).status).toBe(400);
    await stub.fetch("https://registry/live-activity?device=ios-1&id=a1", { method: "DELETE", headers: H });
    expect(await activities(stub)).toEqual([]);
  });

  it("updates on change only, then ends when nothing is live", async () => {
    const stub = room();
    await register(stub, "a1");
    await push(stub, [{ kind: "chats", id: "c1", op: "upsert", hlc: hlc(), set: { title: "Fix login", deviceId: "mac" } }, session("working")]);
    const first = (await activities(stub))[0]?.stamp ?? 0;
    expect(first).toBeGreaterThan(0);
    await push(stub, [session("working")]);
    expect((await activities(stub))[0]?.stamp).toBe(first);
    await push(stub, [session("awaitingInput")]);
    expect((await activities(stub))[0]?.stamp).toBeGreaterThan(first);
    await push(stub, [session("idle")]);
    expect(await activities(stub)).toEqual([]);
  });

  it("a per-session activity ignores other chats", async () => {
    const stub = room();
    await register(stub, "a1", { chatId: "other" });
    await push(stub, [{ kind: "chats", id: "c1", op: "upsert", hlc: hlc(), set: { title: "Fix login" } }, session("working")]);
    expect(await activities(stub)).toEqual([]);
  });
});
