import { useCallback, useEffect, useRef, useState } from "react";
import type { RemoteAccessSnapshot } from "@zeron/proto";
import type { EngineClient } from "@zeron/engine-client";
import { useEngineSession } from "../state/session-provider";
import { useNow } from "../state/hooks";
import { SettingsEngineIndicator } from "../components/settings-engine-indicator";
import {
  createPairingLink,
  getRemoteAccess,
  revokePairingSession,
  sessionIsSelf,
  sessionRows,
  setRemoteAccess,
} from "../lib/remote-access";

/**
 * Remote access settings (desktop settings/remote_access.rs parity): the
 * "Allow remote connections" toggle, the pairing-link mint with copy, and
 * the paired-session list with per-row Revoke; the row that is this
 * browser's own Session (id === session.engine.sessionId) carries the
 * "This browser" badge — the desktop has no self-session (its loopback
 * client is credential-free). Every mutation is followed
 * by a fresh GetRemoteAccess so the page always lands on engine truth;
 * failures render in the strip. Revocation gates the session's next
 * handshake server-side — live connections are not torn down.
 */
export function RemoteAccessSettingsPage() {
  const session = useEngineSession();
  const client = session?.client ?? null;
  const [snapshot, setSnapshot] = useState<RemoteAccessSnapshot | null>(null);
  const [linkUrl, setLinkUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const now = useNow(30_000);
  // One request in flight at a time (the desktop's `busy` gate).
  const inflight = useRef(false);

  const request = useCallback(
    async (run: (client: EngineClient) => Promise<void>) => {
      if (client === null || inflight.current) {
        return;
      }
      inflight.current = true;
      setBusy(true);
      setError(null);
      try {
        await run(client);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        inflight.current = false;
        setBusy(false);
      }
    },
    [client],
  );

  // Mount load, and a full reset when the active engine changes.
  useEffect(() => {
    setSnapshot(null);
    setLinkUrl(null);
    setError(null);
    void request(async (engine) => {
      setSnapshot(await getRemoteAccess(engine));
    });
  }, [request]);

  const enabled = snapshot?.status.enabled ?? false;
  const shownError = error ?? snapshot?.status.error ?? null;

  function refresh() {
    void request(async (engine) => {
      setSnapshot(await getRemoteAccess(engine));
    });
  }

  function toggle() {
    void request(async (engine) => {
      await setRemoteAccess(engine, !enabled);
      setSnapshot(await getRemoteAccess(engine));
    });
  }

  function mintLink() {
    void request(async (engine) => {
      const link = await createPairingLink(engine);
      setLinkUrl(link.url);
      setSnapshot(await getRemoteAccess(engine));
    });
  }

  function revoke(sessionId: string) {
    void request(async (engine) => {
      await revokePairingSession(engine, sessionId);
      setSnapshot(await getRemoteAccess(engine));
    });
  }

  async function copyLink() {
    if (linkUrl === null) {
      return;
    }
    try {
      await navigator.clipboard.writeText(linkUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setError("Could not copy the link — select and copy it by hand.");
    }
  }

  return (
    <div className="settings-page">
      <h1 className="settings-title">Remote access</h1>
      <p className="settings-subtitle">
        Pair your other devices with this engine. Use a trusted network or your own tunnel.
        <SettingsEngineIndicator />
      </p>

      <section className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <span className="settings-row-title">Allow remote connections</span>
            <span className="settings-row-meta">
              {enabled ? "Remote clients can connect with a paired session." : "Only local clients can connect."}
            </span>
          </div>
          <button type="button" className="btn btn-ghost" onClick={refresh} disabled={busy}>
            Refresh
          </button>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            aria-label="Allow remote connections"
            className={`toggle ${enabled ? "toggle-on" : ""}`}
            disabled={busy || snapshot === null}
            onClick={toggle}
          >
            <span className="toggle-thumb" />
          </button>
        </div>
      </section>

      {shownError !== null && (
        <p className="error-strip" role="alert">
          {shownError}
        </p>
      )}

      {(enabled || linkUrl !== null) && (
        <>
          <div className="settings-section-header">
            <h2>Pairing link</h2>
            {enabled && (
              <button type="button" className="btn btn-ghost" disabled={busy} onClick={mintLink}>
                Create pairing link
              </button>
            )}
          </div>
          <section className="settings-card">
            {linkUrl !== null ? (
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title mono settings-url">{linkUrl}</span>
                  <span className="settings-row-meta">Use once within five minutes.</span>
                </div>
                <button type="button" className="btn btn-ghost" onClick={() => void copyLink()}>
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
            ) : (
              <p className="settings-empty">No link yet. Create one and paste it on the other device under Settings → Devices.</p>
            )}
          </section>
        </>
      )}

      <div className="settings-section-header">
        <h2>Paired sessions</h2>
      </div>
      <section className="settings-card">
        {snapshot === null ? (
          <p className="settings-empty">{busy ? "Loading…" : "Nothing to show yet."}</p>
        ) : snapshot.sessions.length === 0 ? (
          <p className="settings-empty">No clients paired yet.</p>
        ) : (
          sessionRows(snapshot, now).map((row) => (
            <div className={`settings-row ${row.revoked ? "settings-row-revoked" : ""}`} key={row.id}>
              <span className={`dot ${row.revoked ? "dot-idle" : "dot-connected"}`} />
              <div className="settings-row-main">
                <span className="settings-row-title">{row.label}</span>
                <span className={`settings-row-meta ${row.revoked ? "settings-meta-danger" : ""}`}>
                  {row.revoked ? "Revoked" : row.lastSeenLabel}
                </span>
              </div>
              {session !== null && sessionIsSelf(row, session.engine) && <span className="badge">This browser</span>}
              {!row.revoked && (
                <button type="button" className="btn btn-danger-ghost" disabled={busy} onClick={() => revoke(row.id)}>
                  Revoke
                </button>
              )}
            </div>
          ))
        )}
      </section>
    </div>
  );
}
