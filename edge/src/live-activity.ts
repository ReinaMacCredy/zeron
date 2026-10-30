import type { FieldValue, Row } from "./registry-core";
import { indicatorOf } from "./push-notify";

export type Which = "all" | "pinned" | "asking";

export interface LiveActivityTarget {
  id: string;
  device: string;
  token: string;
  environment: "production" | "sandbox";
  chatId?: string;
  which: Which;
  alert: boolean;
  endAfter: number;
  pinned: string[];
}

export interface ActivityRow {
  id: string;
  title: string;
  project: string;
  branch?: string;
  state: "working" | "asking" | "done";
  since?: number;
  step?: string;
}

export interface ContentState {
  rows: ActivityRow[];
  running: number;
  asking: number;
}

const str = (v: FieldValue | undefined): string | undefined => (typeof v === "string" ? v : undefined);
const num = (v: FieldValue | undefined): number | undefined => (typeof v === "number" ? v : undefined);

export const stepOf = (preview: string | undefined): string | undefined => {
  const line = preview
    ?.split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => l !== "");
  return line === undefined ? undefined : [...line].slice(0, 80).join("");
};

const projectName = (space: Row | undefined, device: Row | undefined): string => {
  const name = str(space?.fields.name);
  if (name) return name;
  const path = str(space?.fields.path);
  if (path) return path.split("/").filter((p) => p !== "").pop() ?? path;
  return str(device?.fields.name) ?? "";
};

export const liveRows = (
  sessions: Row[],
  chat: (id: string) => Row | undefined,
  space: (id: string) => Row | undefined,
  device: (id: string) => Row | undefined,
  now: number
): ActivityRow[] => {
  const out: ActivityRow[] = [];
  for (const session of sessions) {
    if (session.deleted) continue;
    const indicator = indicatorOf(session.fields, now);
    if (indicator !== "working" && indicator !== "awaiting") continue;
    const c = chat(session.id);
    if (c === undefined || c.deleted || c.fields.archived === true) continue;
    if (str(c.fields.parentChatId) !== undefined || str(c.fields.sidechatSourceChatId) !== undefined) continue;
    const spaceId = str(c.fields.spaceId);
    const deviceId = str(c.fields.deviceId);
    const startedAt = num(session.fields.startedAt);
    const branch = str(c.fields.branch);
    const step = stepOf(str(c.fields.lastMessagePreview));
    out.push({
      id: session.id,
      title: str(c.fields.title)?.trim() || "New session",
      project: projectName(spaceId ? space(spaceId) : undefined, deviceId ? device(deviceId) : undefined),
      ...(branch ? { branch } : {}),
      state: indicator === "awaiting" ? "asking" : "working",
      ...(startedAt !== undefined ? { since: startedAt / 1000 } : {}),
      ...(step ? { step } : {})
    });
  }
  return out.sort((a, b) =>
    a.state !== b.state ? (a.state === "asking" ? -1 : 1) : (a.since ?? 0) - (b.since ?? 0) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  );
};

export const contentFor = (target: Pick<LiveActivityTarget, "chatId" | "which" | "pinned">, rows: ActivityRow[]): ContentState | null => {
  let shown = rows;
  if (target.which === "pinned") shown = shown.filter((r) => target.pinned.includes(r.id));
  if (target.which === "asking") shown = shown.filter((r) => r.state === "asking");
  if (target.chatId !== undefined) {
    const row = shown.find((r) => r.id === target.chatId);
    if (row === undefined) return null;
    return { rows: [row], running: row.state === "working" ? 1 : 0, asking: row.state === "asking" ? 1 : 0 };
  }
  if (shown.length === 0) return null;
  return {
    rows: shown.slice(0, 3),
    running: shown.filter((r) => r.state === "working").length,
    asking: shown.filter((r) => r.state === "asking").length
  };
};

export const finalState = (last: ContentState | null): ContentState => ({
  rows: (last?.rows ?? []).map(({ since: _since, ...row }) => ({ ...row, state: "done" as const })),
  running: 0,
  asking: 0
});

export const newlyAsking = (prev: ContentState | null, next: ContentState): ActivityRow[] => {
  const before = new Set((prev?.rows ?? []).filter((r) => r.state === "asking").map((r) => r.id));
  return next.rows.filter((r) => r.state === "asking" && !before.has(r.id));
};

export const liveActivityPayload = (
  event: "update" | "end",
  state: ContentState,
  timestamp: number,
  options: { alert?: { title: string; body: string }; dismissalDate?: number } = {}
) => ({
  aps: {
    timestamp,
    event,
    "content-state": state,
    ...(options.alert ? { alert: { ...options.alert, sound: "default" } } : {}),
    ...(options.dismissalDate !== undefined ? { "dismissal-date": options.dismissalDate } : {})
  }
});

export const parseTarget = (
  id: string,
  device: string,
  body: Record<string, unknown>
): Omit<LiveActivityTarget, "token" | "environment"> => ({
  id,
  device,
  ...(typeof body.chatId === "string" && body.chatId !== "" ? { chatId: body.chatId } : {}),
  which: body.which === "pinned" || body.which === "asking" ? body.which : "all",
  alert: body.alert !== false,
  endAfter: typeof body.endAfter === "number" && body.endAfter >= 0 ? Math.min(body.endAfter, 4 * 60 * 60) : 15 * 60,
  pinned: Array.isArray(body.pinned) ? body.pinned.filter((p): p is string => typeof p === "string").slice(0, 500) : []
});
