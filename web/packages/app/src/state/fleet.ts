import { useMemo, useSyncExternalStore } from "react";
import type { Chat, Device, Space } from "@zeron/proto";
import {
  EngineRegistry,
  IndexedDbEngineCache,
  encodeScopedId,
  projectRegistrySnapshot,
  type EngineRegistrySnapshot,
  type RowSet,
} from "@zeron/engine-client";
import type { ChatStatus, ConnectivitySlot, WatchCacheSnapshot } from "@zeron/engine-client";
import { EngineStore, engineWsEndpoint, type FleetState, type StoredEngine } from "../lib/engine-store";

/**
 * The origin-scoped engine fleet. `fleetStore` is the pairing storage
 * (redeem/persist/setActive/remove/pinDevice, unchanged shape); the
 * `engineRegistry` is ticket 31's fleet supervisor — one supervised
 * connection per stored engine, all driven simultaneously, merging every
 * engine's rows under scoped ids. The registry follows the store: any
 * pairing change (pair, re-pair, forget) starts or stops supervision
 * immediately, so every existing pairing call site gains live connections
 * without redirection.
 */

export const fleetStore = new EngineStore();

const subscribeFleet = (listener: () => void) => fleetStore.subscribe(listener);
const getFleetSnapshot = () => fleetStore.getSnapshot();

export function useFleet(): FleetState {
  return useSyncExternalStore(subscribeFleet, getFleetSnapshot, getFleetSnapshot);
}

/** The registry singleton: every paired engine, supervised concurrently. */
export const engineRegistry = new EngineRegistry({
  cache: new IndexedDbEngineCache(),
  log: import.meta.env.DEV ? (message, detail) => console.debug("[fleet]", message, detail) : undefined,
});

function registryConfigs(state: FleetState) {
  return state.engines.map((engine) => ({
    key: engine.baseUrl,
    endpoint: engineWsEndpoint(engine.baseUrl),
    credential: engine.credential,
    expectedDeviceId: engine.deviceId,
  }));
}

/** Reconcile the supervised set with whatever the pairing store holds. */
function syncRegistry(): void {
  engineRegistry.sync(registryConfigs(fleetStore.getSnapshot()), fleetStore.getSnapshot().configurationError);
}

fleetStore.subscribe(syncRegistry);
syncRegistry();
// Verified identities pin back into the store so reloads keep verifying
// them (the registry re-adopts `expectedDeviceId` on its next spawn).
engineRegistry.subscribe(() => {
  for (const engine of engineRegistry.getSnapshot().engines) {
    if (engine.state === "connected" && engine.info !== null) {
      fleetStore.pinDevice(engine.key, engine.info.deviceId);
    }
  }
});
if (typeof window !== "undefined") {
  // A cache write in flight must not be lost when the page goes away — park
  // every entry and flush pending writes (`registry.shutdown`).
  for (const event of ["pagehide", "beforeunload"] as const) {
    window.addEventListener(event, () => {
      void engineRegistry.shutdown();
    });
  }
}

const subscribeRegistry = (listener: () => void) => engineRegistry.subscribe(listener);
const getRegistrySnapshot = () => engineRegistry.getSnapshot();

/** The registry's live snapshot: one entry per paired engine + its rows. */
export function useFleetRegistry(): EngineRegistrySnapshot {
  return useSyncExternalStore(subscribeRegistry, getRegistrySnapshot, getRegistrySnapshot);
}

/**
 * Pair a new engine (the store's redeem, with the configuration-error
 * refusal surfaced) — the registry picks the new engine up through its
 * store subscription and starts supervising immediately.
 */
export function pairEngine(pairingUrl: string, label: string): Promise<StoredEngine> {
  return fleetStore.redeemPairingUrl(pairingUrl, label);
}

/** Forget one engine: unpersist it; the registry stops and clears its cache. */
export function forgetEngine(baseUrl: string): void {
  fleetStore.remove(baseUrl);
}

const EMPTY_ROWS: RowSet<never> = { rows: [], loaded: false, error: null };
const NEVER_CONNECTED_SLOT: ConnectivitySlot = { value: null, loaded: false, error: null };
const EMPTY_SNAPSHOT: WatchCacheSnapshot = {
  generation: 0,
  capabilities: [],
  chats: EMPTY_ROWS,
  spaces: EMPTY_ROWS,
  devices: EMPTY_ROWS,
  statuses: EMPTY_ROWS,
  connectivity: NEVER_CONNECTED_SLOT,
};

/**
 * The merged view every fleet-aware surface reads: `projected()` over the
 * registry snapshot, shaped exactly like one engine's `WatchCacheSnapshot`
 * so `chatListRows`/`healedSpaceFilter`/`chatPageRow` and friends operate
 * unchanged — just over more rows. Rows carry scoped ids; a request for one
 * is decoded back to its owning engine at the wire boundary.
 */
export function useFleetSnapshot(): WatchCacheSnapshot {
  const registry = useFleetRegistry();
  const active = useFleet().active;
  return useMemo(() => {
    if (registry.engines.length === 0) {
      return EMPTY_SNAPSHOT;
    }
    const projected = projectRegistrySnapshot(registry);
    return {
      generation: registry.engines.reduce((total, engine) => total + engine.generation, 0),
      capabilities:
        registry.engines.find((engine) => engine.key === active)?.info?.capabilities ?? [],
      chats: mergedRowSet(registry.engines.map((engine) => engine.chats), projected.chats),
      spaces: mergedRowSet(registry.engines.map((engine) => engine.spaces), projected.spaces),
      devices: mergedRowSet(registry.engines.map((engine) => engine.devices), projected.devices),
      statuses: mergedRowSet(registry.engines.map((engine) => engine.sessions), projected.sessions as ChatStatus[]),
      connectivity: NEVER_CONNECTED_SLOT,
    };
  }, [registry, active]);
}

function mergedRowSet<T>(
  parts: readonly RowSet<T>[],
  rows: readonly T[],
): RowSet<T> {
  const loaded = parts.some((part) => part.loaded);
  // A failed PART degrades only its own engine (badge-level, the entry's
  // lastError); the merged list renders whatever rows exist — a parked or
  // reconnecting engine's cached rows must not be blanked by its own
  // stream error (§2.2: last-known rows keep rendering). The error note
  // surfaces only when there is nothing left to show at all.
  const error =
    rows.length === 0 ? (parts.find((part) => part.error !== null)?.error ?? null) : null;
  return { rows, loaded, error };
}

/**
 * The `EngineConnectionState` of the engine a device id resolves to, keyed
 * by engine key — the input to `spaceDeviceTag`'s live-presence override
 * (§2.5: a device backed by a supervised engine reports that engine's
 * connection state, which beats the heartbeat heuristic).
 */
export function engineStatesOf(registry: EngineRegistrySnapshot): Map<string, "connected" | "reconnecting" | "off"> {
  return new Map(registry.engines.map((engine) => [engine.key, engine.state]));
}

/**
 * The fleet's "local device" for group promotion: the ACTIVE engine's own
 * device id, scoped so it matches the projected rows' device ids.
 */
export function fleetLocalDeviceId(registry: EngineRegistrySnapshot, active: string | null): string | null {
  const engine = registry.engines.find((entry) => entry.key === active);
  const deviceId = engine?.info?.deviceId ?? null;
  return engine !== undefined && deviceId !== null ? encodeScopedId(engine.key, deviceId) : null;
}
