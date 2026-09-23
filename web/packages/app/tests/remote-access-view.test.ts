import { describe, expect, it } from "vitest";
import type { EngineClient } from "@zeron/engine-client";
import type { PairedSession, RemoteAccessSnapshot } from "@zeron/proto";
import type { StoredEngine } from "../src/lib/engine-store";
import { formatLastSeen } from "../src/lib/devices";
import type { SessionRow } from "../src/lib/remote-access";
import {
  createPairingLink,
  getRemoteAccess,
  ownSessionId,
  revokePairingSession,
  sessionIsSelf,
  sessionRows,
  setRemoteAccess,
} from "../src/lib/remote-access";

const NOW = 1_800_000_000_000;

describe("formatLastSeen (devices.rs format_last_seen)", () => {
  it("words the buckets the desktop way", () => {
    expect(formatLastSeen(null, NOW)).toBe("never seen");
    expect(formatLastSeen(NOW - 5_000, NOW)).toBe("just now");
    expect(formatLastSeen(NOW - 59_000, NOW)).toBe("just now");
    expect(formatLastSeen(NOW - 60_000, NOW)).toBe("1m ago");
    expect(formatLastSeen(NOW - 5 * 60_000, NOW)).toBe("5m ago");
    expect(formatLastSeen(NOW - 3 * 3_600_000, NOW)).toBe("3h ago");
    expect(formatLastSeen(NOW - 2 * 86_400_000, NOW)).toBe("2d ago");
  });
});

function pairedSession(fields: Partial<PairedSession>): PairedSession {
  return {
    id: "s1",
    label: "Zeron web on Windows",
    createdAt: NOW - 86_400_000,
    lastSeen: NOW - 30_000,
    revokedAt: null,
    ...fields,
  };
}

function snapshot(sessions: PairedSession[]): RemoteAccessSnapshot {
  return {
    status: { enabled: true, configuredEnabled: true, address: "0.0.0.0:27655", source: "default", error: null },
    sessions,
  };
}

function sessionRow(fields: Partial<SessionRow> = {}): SessionRow {
  return { id: "s1", label: "Zeron web on Windows", revoked: false, lastSeenLabel: null, ...fields };
}

function storedEngine(fields: Partial<StoredEngine> = {}): StoredEngine {
  return {
    baseUrl: "http://127.0.0.1:27655",
    credential: "credential",
    label: "Zeron web on Windows",
    sessionId: "s1",
    pairedAt: NOW - 86_400_000,
    deviceId: null,
    ...fields,
  };
}

describe("sessionRows", () => {
  it("falls back to a generic label for unlabeled sessions", () => {
    const rows = sessionRows(snapshot([pairedSession({ label: "" })]), NOW);
    expect(rows[0]!.label).toBe("Paired device");
    expect(rows[0]!.revoked).toBe(false);
    expect(rows[0]!.lastSeenLabel).toBe("Last seen just now");
  });

  it("marks revoked rows and drops their last-seen line", () => {
    const rows = sessionRows(snapshot([pairedSession({ revokedAt: NOW - 1_000 })]), NOW);
    expect(rows[0]!.revoked).toBe(true);
    expect(rows[0]!.lastSeenLabel).toBe(null);
  });
});

describe("sessionIsSelfMatchesStoredSessionId", () => {
  it("matches the row whose id equals the stored engine's sessionId", () => {
    expect(sessionIsSelf(sessionRow({ id: "s1" }), storedEngine())).toBe(true);
  });

  it("leaves every other row unmatched", () => {
    expect(sessionIsSelf(sessionRow({ id: "s2" }), storedEngine())).toBe(false);
  });

  it("never matches when the stored engine has no session id", () => {
    const stored = storedEngine({ sessionId: null as unknown as string });
    expect(ownSessionId(stored)).toBe(null);
    expect(sessionIsSelf(sessionRow({ id: "s1" }), stored)).toBe(false);
  });

  it("still matches a revoked self row — the badge outlives the session", () => {
    expect(sessionIsSelf(sessionRow({ id: "s1", revoked: true }), storedEngine())).toBe(true);
  });
});

/** A call-recording fake EngineClient — the protocol-boundary check. */
function fakeClient(replies: Record<string, unknown>) {
  const calls: { method: string; params: unknown }[] = [];
  const client = {
    async call(method: string, params: unknown): Promise<unknown> {
      calls.push({ method, params });
      const reply = replies[method];
      if (reply === undefined) {
        throw new Error(`unknown method: ${method}`);
      }
      return reply;
    },
  } as unknown as EngineClient;
  return { client, calls };
}

describe("remote access RPC wrappers", () => {
  it("reads the snapshot with GetRemoteAccess and no params", async () => {
    const { client, calls } = fakeClient({ GetRemoteAccess: snapshot([]) });
    expect(await getRemoteAccess(client)).toEqual(snapshot([]));
    expect(calls).toEqual([{ method: "GetRemoteAccess", params: {} }]);
  });

  it("sends the toggle as SetRemoteAccess {enabled}", async () => {
    const { client, calls } = fakeClient({ SetRemoteAccess: snapshot([]) });
    await setRemoteAccess(client, true);
    expect(calls).toEqual([{ method: "SetRemoteAccess", params: { enabled: true } }]);
  });

  it("revokes by sessionId (revocation gates the next handshake server-side)", async () => {
    const { client, calls } = fakeClient({ RevokePairingSession: snapshot([]) });
    await revokePairingSession(client, "s1");
    expect(calls).toEqual([{ method: "RevokePairingSession", params: { sessionId: "s1" } }]);
  });

  it("mints a pairing link", async () => {
    const link = { url: "http://host:27655/pair#token=abc", expiresAt: NOW + 300_000 };
    const { client, calls } = fakeClient({ CreatePairingLink: link });
    expect(await createPairingLink(client)).toEqual(link);
    expect(calls).toEqual([{ method: "CreatePairingLink", params: {} }]);
  });
});
