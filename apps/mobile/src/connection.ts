import Tailcat from '../modules/my-module';

type Session = {
  baseUrl: string;
  token: string;
  expiresAt: number;
  principal: { profileId: string; deviceId: string };
};

export type Engine = { deviceId: string; displayName: string | null; owner: boolean };

type Pending = {
  resolve?: (value: unknown) => void;
  reject?: (error: Error) => void;
  onItem?: (item: unknown) => void;
  onError?: (error: Error) => void;
  timer?: ReturnType<typeof setTimeout>;
};

import { utf8Decode, utf8Encode } from './utf8';
export { utf8Decode, utf8Encode } from './utf8';

const rpcHeader = utf8Encode('{"s":"rpc","k":"rpc"}');
const echoFrame = frame(utf8Encode('{"s":"echo","k":"echo"}'), new Uint8Array());

function frame(header: Uint8Array, payload: Uint8Array): ArrayBuffer {
  const bytes = new Uint8Array(1 + header.length + payload.length);
  bytes[0] = header.length; // Both fixed client headers are shorter than 128 bytes.
  bytes.set(header, 1);
  bytes.set(payload, header.length + 1);
  return bytes.buffer;
}

function decodeFrame(data: ArrayBuffer): { kind: string; payload: string } {
  const bytes = new Uint8Array(data);
  let length = 0, shift = 0, offset = 0, byte = 0;
  do {
    if (offset >= bytes.length || shift > 28) throw new Error('Invalid device frame');
    byte = bytes[offset++];
    length |= (byte & 0x7f) << shift;
    shift += 7;
  } while (byte & 0x80);
  if (offset + length > bytes.length) throw new Error('Invalid device frame');
  const header = JSON.parse(utf8Decode(bytes.subarray(offset, offset + length))) as { k: string };
  return { kind: header.k, payload: utf8Decode(bytes.subarray(offset + length)) };
}

function parseSession(json: string): Session {
  const value = JSON.parse(json) as Session;
  if (!value.baseUrl?.startsWith('http://127.0.0.1:') || !value.token ||
      value.expiresAt <= Date.now() / 1000 || !value.principal?.profileId || !value.principal?.deviceId) {
    throw new Error('Peer returned an invalid connection session');
  }
  return value;
}

export class Connection {
  readonly profileId: string;
  readonly deviceId: string;
  readonly baseUrl: string;
  engines: Engine[] = [];
  hostDeviceId: string | null = null;
  private session: Session;
  private socket: WebSocket | null = null;
  private opening: Promise<WebSocket> | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private lastInbound = 0;
  private lastHostProof = 0;
  private echoSeen = false;
  private closed = false;
  private generation = 0;

  constructor(session: Session) {
    this.session = session;
    this.baseUrl = session.baseUrl;
    this.profileId = session.principal.profileId;
    this.deviceId = session.principal.deviceId;
  }

  private async renew(force = false) {
    if (!force && this.session.expiresAt > Date.now() / 1000 + 60) return;
    if (!Tailcat) throw new Error('Tailcat is unavailable');
    const next = parseSession(JSON.stringify({ ...JSON.parse(await Tailcat.renew()), baseUrl: this.baseUrl }));
    if (next.principal.profileId !== this.profileId || next.principal.deviceId !== this.deviceId) {
      throw new Error('Peer returned a different device identity');
    }
    this.session = next;
  }

  deviceSocketUrl(): string {
    if (!this.hostDeviceId) throw new Error('No host engine is available');
    const url = new URL(`device/${encodeURIComponent(this.hostDeviceId)}/ws`, this.baseUrl + '/');
    url.protocol = 'ws:';
    url.searchParams.set('role', 'client');
    url.searchParams.set('connId', `${Date.now()}-${Math.random().toString(36).slice(2)}`);
    url.searchParams.set('token', this.session.token);
    return url.toString();
  }

  async discoverEngines(): Promise<Engine[]> {
    const response = await this.request('pair/engines');
    if (!response.ok) throw new Error(`Engine discovery failed (${response.status})`);
    const engines = await response.json();
    if (!Array.isArray(engines) || !engines.every(engine => typeof engine.deviceId === 'string' && engine.owner === true)) {
      throw new Error('Peer returned an invalid engine list');
    }
    this.engines = engines;
    if (!this.hostDeviceId || !engines.some(engine => engine.deviceId === this.hostDeviceId)) {
      this.drop(new Error('Host engine changed'));
      this.hostDeviceId = engines[0]?.deviceId ?? null;
    }
    return engines;
  }

  async verifyHost(): Promise<void> {
    if (!this.engines.length) throw new Error('No host engine is paired');
    let lastError: unknown;
    for (const engine of this.engines) {
      this.selectHostDevice(engine.deviceId);
      try {
        const info = await this.call('EngineInfo', {});
        if (!info || typeof info !== 'object') throw new Error('Host engine returned invalid information');
        return;
      } catch (error) {
        lastError = error;
        this.drop(error instanceof Error ? error : new Error('Host engine is unavailable'));
      }
    }
    throw lastError;
  }

  selectHostDevice(deviceId: string) {
    if (!this.engines.some(engine => engine.deviceId === deviceId)) throw new Error('Unknown host engine');
    if (this.hostDeviceId !== deviceId) this.drop(new Error('Host engine changed'));
    this.hostDeviceId = deviceId;
  }

  async request(path: string, init: RequestInit = {}): Promise<Response> {
    if (this.closed) throw new Error('Connection is closed');
    if (path.startsWith('/') || path.includes('..') || path.includes('://')) throw new Error('Invalid peer path');
    await this.renew();
    const send = () => fetch(new URL(path, this.baseUrl + '/').toString(), {
      ...init, headers: { ...Object.fromEntries(new Headers(init.headers).entries()), Authorization: `Bearer ${this.session.token}` },
    });
    let response = await send();
    if (response.status === 401) {
      await this.renew(true);
      response = await send();
    }
    if (response.status === 401 || response.status === 403) throw new Error('Device access was revoked or expired');
    return response;
  }

  private async open(): Promise<WebSocket> {
    if (this.closed) throw new Error('Connection is closed');
    if (this.socket?.readyState === WebSocket.OPEN) return this.socket;
    if (this.opening) return this.opening;
    this.opening = (async () => {
      const generation = this.generation;
      await this.renew();
      const socket = new WebSocket(this.deviceSocketUrl());
      socket.binaryType = 'arraybuffer';
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => { socket.close(); reject(new Error('Peer connection timed out')); }, 10000);
        socket.onopen = () => { clearTimeout(timer); resolve(); };
        socket.onerror = () => { clearTimeout(timer); reject(new Error('Peer connection failed')); };
      });
      if (this.closed || this.generation !== generation) { socket.close(); throw new Error('Connection changed'); }
      this.socket = socket;
      this.lastInbound = Date.now();
      this.lastHostProof = Date.now();
      this.echoSeen = false;
      socket.onmessage = event => this.receive(event.data);
      socket.onclose = () => { if (this.socket === socket) this.drop(new Error('Peer connection closed')); };
      socket.onerror = () => { if (this.socket === socket) this.drop(new Error('Peer connection failed')); };
      try { socket.send(echoFrame); }
      catch { this.drop(new Error('Peer connection failed')); throw new Error('Peer connection failed'); }
      this.heartbeat = setInterval(() => {
        if (Date.now() - this.lastInbound > 25000 || (this.echoSeen && Date.now() - this.lastHostProof > 20000)) {
          this.drop(new Error('Peer connection lost'));
          return;
        }
        try {
          socket.send('ping');
          socket.send(echoFrame);
        } catch { this.drop(new Error('Peer connection failed')); }
      }, 10000);
      return socket;
    })();
    const opening = this.opening;
    try { return await opening; }
    finally { if (this.opening === opening) this.opening = null; }
  }

  private receive(data: unknown) {
    this.lastInbound = Date.now();
    if (!(data instanceof ArrayBuffer)) return;
    try {
      const { kind, payload } = decodeFrame(data);
      if (kind === 'echo') { this.echoSeen = true; this.lastHostProof = Date.now(); return; }
      if (kind === ' relay') { this.drop(new Error('Host is offline')); return; }
      if (kind !== 'rpc') return;
      this.echoSeen = true;
      this.lastHostProof = Date.now();
      for (const line of payload.split('\n').filter(Boolean)) {
        const reply = JSON.parse(line) as { id: number; ok?: unknown; item?: unknown; err?: string; done?: boolean };
        const pending = this.pending.get(reply.id);
        if (!pending) continue;
        if (reply.err) {
          this.finish(reply.id);
          const error = new Error(reply.err);
          pending.reject?.(error);
          pending.onError?.(error);
        } else if (Object.prototype.hasOwnProperty.call(reply, 'item')) {
          pending.onItem?.(reply.item);
        } else if (reply.done) {
          this.finish(reply.id);
          if (!pending.onItem) pending.reject?.(new Error('Peer ended a unary call without a result'));
        } else if (Object.prototype.hasOwnProperty.call(reply, 'ok') && !pending.onItem) {
          this.finish(reply.id);
          pending.resolve?.(reply.ok);
        }
      }
    } catch (error) { this.drop(error instanceof Error ? error : new Error('Invalid peer reply')); }
  }

  private finish(id: number) {
    const pending = this.pending.get(id);
    if (pending?.timer) clearTimeout(pending.timer);
    this.pending.delete(id);
  }

  private drop(error: Error) {
    this.generation++;
    this.opening = null;
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.heartbeat = null;
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      socket.onclose = null;
      socket.onerror = null;
      socket.onmessage = null;
      if (socket.readyState !== WebSocket.CLOSED) socket.close();
    }
    for (const [id, pending] of this.pending) {
      this.finish(id);
      pending.reject?.(error);
      pending.onError?.(error);
    }
  }

  async call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const socket = await this.open();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.drop(new Error(`${method} timed out`)), 10000);
      this.pending.set(id, { resolve, reject, timer });
      try { socket.send(frame(rpcHeader, utf8Encode(JSON.stringify({ id, method, params })))); }
      catch (error) { this.finish(id); reject(error); }
    });
  }

  async subscribe(method: string, params: Record<string, unknown>, onItem: (item: unknown) => void,
    onError: (error: Error) => void = () => {}): Promise<() => void> {
    const socket = await this.open();
    const id = this.nextId++;
    this.pending.set(id, { onItem, onError });
    try { socket.send(frame(rpcHeader, utf8Encode(JSON.stringify({ id, method, params })))); }
    catch (error) { this.finish(id); throw error; }
    return () => {
      if (!this.pending.has(id)) return;
      this.finish(id);
      if (socket.readyState === WebSocket.OPEN) {
        try { socket.send(frame(rpcHeader, utf8Encode(JSON.stringify({ id, cancel: true })))); }
        catch { this.drop(new Error('Peer connection failed')); }
      }
    };
  }

  disconnect() {
    this.closed = true;
    this.drop(new Error('Connection closed'));
    Tailcat?.disconnect();
  }
}

let active: Connection | null = null;

export async function connect(invitation: string): Promise<Connection> {
  if (!Tailcat) throw new Error('Install the Android app with Tailcat support');
  active?.disconnect();
  active = null;
  try {
    const connection = new Connection(parseSession(await Tailcat.connect(invitation)));
    await connection.discoverEngines();
    await connection.verifyHost();
    active = connection;
    return connection;
  } catch (error) {
    Tailcat.disconnect();
    throw error;
  }
}

export async function restore(): Promise<Connection | null> {
  if (!Tailcat) return null;
  active?.disconnect();
  active = null;
  const session = await Tailcat.restore();
  if (!session) return null;
  try {
    const connection = new Connection(parseSession(session));
    await connection.discoverEngines();
    await connection.verifyHost();
    active = connection;
    return connection;
  } catch (error) {
    Tailcat.disconnect();
    throw error;
  }
}
