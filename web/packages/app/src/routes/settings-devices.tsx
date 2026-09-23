import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Icon } from "@zeron/icons";
import { methods, type EngineEntrySnapshot } from "@zeron/engine-client";
import type { Device } from "@zeron/proto";
import { useEngineSession } from "../state/session-provider";
import { forgetEngine, useFleet, useFleetRegistry, fleetStore } from "../state/fleet";
import { useEngineStatus, useNow, useWatchSnapshot } from "../state/hooks";
import {
  BtnGhost,
  BtnPrimary,
  Dialog,
  DialogCard,
  DialogField,
  DialogTitle,
} from "../components/ui/Dialog";
import { webDeviceLabel, engineHost, type StoredEngine } from "../lib/engine-store";
import { describeRedeemError } from "../lib/pairing-errors";
import { engineConnection } from "../lib/settings-engine";
import {
  lastSeenOnline,
  formatLastSeenAt,
  platformGlyph,
  platformLabel,
  presenceDot,
  shortId,
  type EngineConnection,
} from "../lib/devices";

/**
 * Devices settings (desktop settings/devices.rs parity): the device registry
 * of the connected engine — one row per device that has paired with it, with
 * the platform tile's corner presence dot, the meta line (platform · version
 * · connection · last seen · added · the click-to-copy id chip), Rename (via
 * the Mutate renameDevice op) and the pairing box that redeems a pairing URL
 * through the fleet store — the page's one paste entry (the `/pair` landing
 * is the other, for token URLs). Above the device rows sits the engines
 * card: one row per engine this browser paired, folded here from the
 * deleted web-only user-menu Engines drawer (ticket 45) — connection state
 * dot + label, the engine identity line, "Pair again" when the engine
 * parked (a revoked Session re-pairs through `/pair`), and Forget.
 *
 * Web mapping of the desktop's multi-engine registry concepts: the rows come
 * from the connected engine's WatchDevices; the "engine-backed" row is the
 * one whose id matches the connected engine's own device (its presence is
 * the live connection state), a row matching a parked fleet engine is
 * engine-backed-off (Forget removes it from the fleet); every other row
 * falls back to the last-seen window.
 */

interface RenameDialog {
  readonly deviceId: string;
  readonly name: string;
}

export function DevicesSettingsPage() {
  const session = useEngineSession();
  const client = session?.client ?? null;
  const status = useEngineStatus(session);
  const fleet = useFleet();
  const registry = useFleetRegistry();
  const snapshot = useWatchSnapshot(session);
  const now = useNow(15_000);
  const [pairingUrl, setPairingUrl] = useState("");
  const [pairingBusy, setPairingBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [rename, setRename] = useState<RenameDialog | null>(null);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // A new engine (a successful pair) rebuilds the session; reset the page.
  useEffect(() => {
    setError(null);
    setCopied(null);
    setRename(null);
  }, [session]);

  useEffect(() => {
    return () => {
      if (copyTimer.current !== null) {
        clearTimeout(copyTimer.current);
      }
    };
  }, []);

  const devices = snapshot?.devices.rows ?? [];
  const localDeviceId = session?.client.engineInfo?.deviceId ?? null;
  const activeEngine = fleet.engines.find((engine) => engine.baseUrl === fleet.active) ?? null;
  // One registry entry per stored engine, keyed by its baseUrl.
  const registryByEngine = new Map(registry.engines.map((entry) => [entry.key, entry]));

  /** The live engine connection behind a row, or null (last-seen fallback). */
  function rowConnection(deviceId: string): EngineConnection | null {
    if (deviceId === localDeviceId) {
      if (status === null) {
        return null;
      }
      switch (status.state) {
        case "connected":
          return "connected";
        case "connecting":
        case "reconnecting":
          return "reconnecting";
        default:
          return "off";
      }
    }
    // A row backed by a parked engine this client knows: engine-backed, off.
    const parked = fleet.engines.find(
      (engine) => engine.deviceId === deviceId && engine.baseUrl !== fleet.active,
    );
    return parked === undefined ? null : "off";
  }

  /** The fleet engine a non-local engine-backed row forgets from, if any. */
  function forgetTarget(deviceId: string): string | null {
    const parked = fleet.engines.find(
      (engine) => engine.deviceId === deviceId && engine.baseUrl !== fleet.active,
    );
    return parked?.baseUrl ?? null;
  }

  function pair() {
    if (pairingBusy) {
      return;
    }
    const url = pairingUrl.trim();
    if (url.length === 0) {
      return;
    }
    setPairingBusy(true);
    setError(null);
    void (async () => {
      try {
        await fleetStore.redeemPairingUrl(url, webDeviceLabel());
        setPairingUrl("");
      } catch (cause) {
        setError(describeRedeemError(cause));
      } finally {
        setPairingBusy(false);
      }
    })();
  }

  function forget(baseUrl: string) {
    fleetStore.remove(baseUrl);
  }

  /** The dialog closes FIRST, then the trimmed name decides whether an RPC fires (submit_rename's silent-swallow quirk). */
  function submitRename(name: string) {
    const deviceId = rename?.deviceId;
    setRename(null);
    const trimmed = name.trim();
    if (deviceId === undefined || trimmed.length === 0 || client === null) {
      return;
    }
    void (async () => {
      try {
        await client.call(methods.MUTATE, { op: "renameDevice", deviceId, name: trimmed });
      } catch (cause) {
        setError(`Rename failed: ${cause instanceof Error ? cause.message : String(cause)}`);
      }
    })();
  }

  function copyId(deviceId: string) {
    void navigator.clipboard.writeText(deviceId).catch(() => {});
    setCopied(deviceId);
    if (copyTimer.current !== null) {
      clearTimeout(copyTimer.current);
    }
    copyTimer.current = setTimeout(() => setCopied(null), 1500);
  }

  const count = devices.length;

  return (
    <div className="settings-page">
      <h1 className="settings-title">
        Devices{count > 0 && <span className="settings-title-count">{count}</span>}
      </h1>
      <p className="settings-subtitle">Connect and manage engines.</p>

      {error !== null && (
        <p className="error-strip" role="alert" onClick={() => setError(null)}>
          {error}
        </p>
      )}

      <section className="settings-card settings-pairing-box">
        <form
          className="settings-pairing-row"
          onSubmit={(event) => {
            event.preventDefault();
            pair();
          }}
        >
          <input
            className="input mono"
            type="text"
            placeholder="Paste a pairing URL"
            value={pairingUrl}
            onChange={(event) => setPairingUrl(event.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button type="submit" className="btn btn-solid">
            {pairingBusy ? "Connecting…" : "Connect"}
          </button>
        </form>
        <p className="settings-pairing-hint">
          Create a pairing link in the engine's Remote access settings, then paste it here.
        </p>
      </section>

      <div className="settings-section-header">
        <h2>Engines</h2>
      </div>
      <section className="settings-card">
        {fleet.engines.length === 0 ? (
          <p className="settings-empty">No engines paired yet. Pair one above.</p>
        ) : (
          fleet.engines.map((engine) => (
            <EngineRow
              key={engine.baseUrl}
              engine={engine}
              entry={registryByEngine.get(engine.baseUrl) ?? null}
            />
          ))
        )}
      </section>

      <section className="settings-card">
        {count === 0 ? (
          <p className="settings-empty settings-empty-devices">No devices registered</p>
        ) : (
          devices.map((device, ix) => (
            <DeviceRow
              key={device.id}
              device={device}
              first={ix === 0}
              isLocal={device.id === localDeviceId}
              connection={rowConnection(device.id)}
              online={lastSeenOnline(device.lastSeenAt, now)}
              copied={copied === device.id}
              now={now}
              forgetBaseUrl={forgetTarget(device.id)}
              onCopyId={() => copyId(device.id)}
              onForget={() => {
                const target = forgetTarget(device.id);
                if (target !== null) {
                  forget(target);
                }
              }}
              onRename={() => setRename({ deviceId: device.id, name: device.name })}
            />
          ))
        )}
      </section>

      {rename !== null && (
        <RenameDeviceDialog dialog={rename} onCancel={() => setRename(null)} onSubmit={submitRename} />
      )}
    </div>
  );
}

interface EngineRowProps {
  readonly engine: StoredEngine;
  readonly entry: EngineEntrySnapshot | null;
}

/**
 * One paired engine's row — the drawer's per-engine row folded here
 * (ticket 45), minus the urgent-chat dot (the sidebar owns urgency).
 * The meta line is the connection label · identity (`Engine {shortId}`,
 * or "Identity unverified" before the first verified connect); "Pair
 * again" appears only on a parked engine and re-enters the pairing flow
 * at `/pair`, where a fresh URL redeems into a new Session.
 */
function EngineRow({ engine, entry }: EngineRowProps) {
  const navigate = useNavigate();
  const connection = engineConnection(entry);

  return (
    <div className="settings-row">
      <span className={`dot ${connection.dot}`} />
      <div className="settings-row-main">
        <span className="settings-row-title">{engineHost(engine.baseUrl)}</span>
        <span className="settings-meta-line">
          {connection.label}
          <span className="settings-meta-dot" aria-hidden="true">·</span>
          {engine.deviceId !== null ? `Engine ${engine.deviceId.slice(0, 8)}` : "Identity unverified"}
        </span>
      </div>
      {connection.pairable && (
        <button type="button" className="btn btn-ghost" onClick={() => void navigate({ to: "/pair" })}>
          Pair again
        </button>
      )}
      <RemoveEngineButton engine={engine} />
    </div>
  );
}

function RemoveEngineButton({ engine }: { engine: StoredEngine }) {
  const [confirming, setConfirming] = useState(false);
  if (!confirming) {
    return (
      <button type="button" className="btn btn-ghost" onClick={() => setConfirming(true)}>
        Forget
      </button>
    );
  }
  return (
    <button type="button" className="btn btn-danger-ghost" onClick={() => forgetEngine(engine.baseUrl)}>
      Forget?
    </button>
  );
}

function DeviceRow(props: {
  readonly device: Device;
  readonly first: boolean;
  readonly isLocal: boolean;
  readonly connection: EngineConnection | null;
  readonly online: boolean;
  readonly copied: boolean;
  readonly now: number;
  readonly forgetBaseUrl: string | null;
  readonly onCopyId: () => void;
  readonly onForget: () => void;
  readonly onRename: () => void;
}) {
  const device = props.device;
  const dot = presenceDot(props.connection, props.online);
  const version = device.version !== null && device.version !== undefined && device.version.length > 0
    ? device.version
    : null;
  return (
    <div className="settings-row device-row">
      <div className="row-tile device-tile" aria-hidden="true">
        <Icon name={platformGlyph(device.platform)} size={16} className="row-tile-icon" />
        <span className={`presence-dot presence-${dot}`} />
      </div>
      <div className="settings-row-main">
        <span className="settings-row-title">{device.name}</span>
        <span className="settings-meta-line">
          {platformLabel(device.platform)}
          {version !== null && (
            <>
              <span className="settings-meta-dot" aria-hidden="true">·</span>
              {`v${version}`}
            </>
          )}
          {props.connection !== null && (
            <>
              <span className="settings-meta-dot" aria-hidden="true">·</span>
              {props.connection === "connected"
                ? "Connected"
                : props.connection === "reconnecting"
                  ? "Reconnecting"
                  : "Off"}
            </>
          )}
          {!props.online && (
            <>
              <span className="settings-meta-dot" aria-hidden="true">·</span>
              {`Last seen ${formatLastSeenAt(device.lastSeenAt, props.now)}`}
            </>
          )}
          {device.createdAt !== null && device.createdAt !== undefined && (
            <>
              <span className="settings-meta-dot" aria-hidden="true">·</span>
              {`Added ${formatLastSeenAt(device.createdAt, props.now)}`}
            </>
          )}
          <span className="settings-meta-dot" aria-hidden="true">·</span>
          <button
            type="button"
            className={`id-chip ${props.copied ? "id-chip-copied" : ""}`}
            onClick={props.onCopyId}
            aria-label={`Copy device id ${device.id}`}
          >
            {props.copied ? "Copied" : shortId(device.id)}
          </button>
        </span>
      </div>
      {props.isLocal && <span className="badge">This device</span>}
      {props.forgetBaseUrl !== null && (
        <button type="button" className="btn btn-ghost" onClick={props.onForget}>
          Forget
        </button>
      )}
      <button type="button" className="btn btn-ghost device-rename" onClick={props.onRename}>
        <Icon name="pen" size={14} />
        Rename
      </button>
    </div>
  );
}

/**
 * The rename dialog (devices.rs render_rename_dialog): the shared
 * `ui/Dialog` family — `RbDialog`'s scrim swallows presses without
 * dismissing (its `disablePointerDismissal` carries the old hand-roll's
 * parity quirk for free) and the phone arm is the family's bottom sheet
 * (the old fixed-centered card squashed at ≤768px). Cancel, Rename, and
 * Enter close it, as before; Escape now cancels too — the one gained
 * path, matching the shared dialogs (documented deviation). Exported
 * for the mounted family test (tests/settings-dialogs.test.ts).
 */
export function RenameDeviceDialog(props: {
  readonly dialog: RenameDialog;
  readonly onCancel: () => void;
  readonly onSubmit: (name: string) => void;
}) {
  const [name, setName] = useState(props.dialog.name);
  const inputRef = useRef<HTMLInputElement | null>(null);
  return (
    <Dialog ariaLabel="Rename device" onClose={props.onCancel} initialFocus={inputRef}>
      <DialogCard>
        <DialogTitle>Rename device</DialogTitle>
        <form
          className="dialog-form-rows"
          onSubmit={(event) => {
            event.preventDefault();
            props.onSubmit(name);
          }}
        >
          <DialogField>
            <input
              ref={inputRef}
              type="text"
              placeholder="Device name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              autoComplete="off"
              aria-label="Device name"
            />
          </DialogField>
          <div className="dialog-actions-row">
            <BtnGhost type="button" onClick={props.onCancel}>
              Cancel
            </BtnGhost>
            <BtnPrimary type="submit">Rename</BtnPrimary>
          </div>
        </form>
      </DialogCard>
    </Dialog>
  );
}
