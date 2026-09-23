import { methods, type EngineClient } from "@zeron/engine-client";
import type { PairedSession, PairingLink, RemoteAccessSnapshot } from "@zeron/proto";
import type { StoredEngine } from "./engine-store";
import { formatLastSeen } from "./devices";

/**
 * Remote access settings — the web peer of the desktop's
 * settings/remote_access.rs page. The pure pieces (last-seen wording, row
 * derivation) are ported verbatim; the call wrappers pin the wire contract
 * (every mutation's reply is a fresh `RemoteAccessSnapshot`, and the page
 * still refetches after each mutation, matching the desktop).
 */

export function getRemoteAccess(client: EngineClient): Promise<RemoteAccessSnapshot> {
  return client.call<RemoteAccessSnapshot>(methods.GET_REMOTE_ACCESS, {});
}

export function setRemoteAccess(client: EngineClient, enabled: boolean): Promise<RemoteAccessSnapshot> {
  return client.call<RemoteAccessSnapshot>(methods.SET_REMOTE_ACCESS, { enabled });
}

export function createPairingLink(client: EngineClient): Promise<PairingLink> {
  return client.call<PairingLink>(methods.CREATE_PAIRING_LINK, {});
}

export function revokePairingSession(client: EngineClient, sessionId: string): Promise<RemoteAccessSnapshot> {
  return client.call<RemoteAccessSnapshot>(methods.REVOKE_PAIRING_SESSION, { sessionId });
}

/** One paired-session row, ready to draw. */
export interface SessionRow {
  readonly id: string;
  /** The grant label, or the desktop's fallback for an unlabeled session. */
  readonly label: string;
  readonly revoked: boolean;
  /** "Last seen …" — meaningless on a revoked row; null then. */
  readonly lastSeenLabel: string | null;
}

export function sessionRows(snapshot: RemoteAccessSnapshot, now: number): SessionRow[] {
  return snapshot.sessions.map((session: PairedSession) => {
    const revoked = session.revokedAt !== null;
    return {
      id: session.id,
      label: session.label.length > 0 ? session.label : "Paired device",
      revoked,
      lastSeenLabel: revoked ? null : `Last seen ${formatLastSeen(session.lastSeen, now)}`,
    };
  });
}

/**
 * The routed engine's stored Session id — the row this browser should mark
 * as its own. Null when the id is missing (shape-drifted storage); a null
 * id never matches a row. The desktop has no counterpart: its loopback
 * client is credential-free and holds no Session at all.
 */
export function ownSessionId(stored: StoredEngine): string | null {
  return typeof stored.sessionId === "string" ? stored.sessionId : null;
}

/** True when `row` is the stored engine's own paired Session. */
export function sessionIsSelf(row: SessionRow, stored: StoredEngine): boolean {
  return row.id === ownSessionId(stored);
}
