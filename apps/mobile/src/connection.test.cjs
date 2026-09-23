const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');

function load(sourcePath, dependencies, globals = {}) {
  const source = fs.readFileSync(path.join(__dirname, sourcePath), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, esModuleInterop: true, target: ts.ScriptTarget.ES2020 } }).outputText;
  const exports = {};
  vm.runInNewContext(code, {
    exports, require: name => dependencies[name], URL, Headers, Uint8Array, ArrayBuffer,
    TextEncoder, TextDecoder, encodeURIComponent, decodeURIComponent, setTimeout,
    clearTimeout, setInterval, clearInterval, Date, Math, JSON, Error, fetch: (...args) => global.fetch(...args), ...globals,
  });
  return exports;
}

function makeFrame(kind, payload) {
  const header = Buffer.from(JSON.stringify({ s: kind, k: kind }));
  const body = Buffer.from(payload);
  return Uint8Array.from([header.length, ...header, ...body]).buffer;
}

async function waitFor(check) {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await new Promise(resolve => setTimeout(resolve, 1));
  }
  throw new Error('Timed out waiting for socket');
}

function setup() {
  const sockets = [];
  class Socket {
    static OPEN = 1;
    static CLOSED = 3;
    readyState = 0;
    sent = [];
    constructor(url) {
      this.url = url;
      sockets.push(this);
      setImmediate(() => { if (this.readyState === 0) { this.readyState = 1; this.onopen?.(); } });
    }
    send(data) { this.sent.push(data); }
    close() { this.readyState = 3; setImmediate(() => this.onclose?.()); }
  }
  const tailcat = { disconnect() {}, async renew() { throw new Error('Unexpected renewal'); } };
  const utf8 = load('utf8.ts', {});
  const module = load('connection.ts', { '../modules/my-module': tailcat, './utf8': utf8 }, { WebSocket: Socket });
  const session = { baseUrl: 'http://127.0.0.1:7333', token: 'old-token', expiresAt: Math.floor(Date.now() / 1000) + 900,
    principal: { profileId: 'profile', deviceId: 'phone' } };
  const connection = new module.Connection(session);
  connection.engines = [{ deviceId: 'host', displayName: 'Desktop', owner: true }, { deviceId: 'host2', displayName: 'Laptop', owner: true }];
  connection.hostDeviceId = 'host';
  return { connection, sockets, tailcat, utf8 };
}

test('ControlRpc targets the host, routes unary and stream replies, and cancels streams', async () => {
  const { connection, sockets, utf8 } = setup();
  try {
    const call = connection.call('EngineInfo', {});
    await waitFor(() => sockets[0]?.sent.length >= 2);
    const socket = sockets[0];
    assert.match(socket.url, /device\/host\/ws\?role=client/);
    assert.doesNotMatch(socket.url, /device\/phone\/ws/);
    assert.equal(new Uint8Array(socket.sent[1])[0], 21); // fixed RPC header length
    socket.onmessage({ data: makeFrame('rpc', JSON.stringify({ id: 1, ok: { name: 'Desktop' } }) + '\n') });
    assert.equal((await call).name, 'Desktop');

    const items = [];
    const cancel = await connection.subscribe('WatchChats', {}, item => items.push(item));
    socket.onmessage({ data: makeFrame('rpc', JSON.stringify({ id: 2, item: { id: 'chat' } }) + '\n') });
    assert.equal(items[0].id, 'chat');
    cancel();
    const cancelBytes = new Uint8Array(socket.sent.at(-1));
    assert.deepEqual(JSON.parse(utf8.utf8Decode(cancelBytes.subarray(22))), { id: 2, cancel: true });
  } finally { connection.disconnect(); }
});

test('stale socket close cannot kill a newly selected host', async () => {
  const { connection, sockets } = setup();
  try {
    const first = connection.call('EngineInfo', {});
    await waitFor(() => sockets[0]?.sent.length >= 2);
    sockets[0].onmessage({ data: makeFrame('rpc', JSON.stringify({ id: 1, ok: {} }) + '\n') });
    await first;
    const staleClose = sockets[0].onclose;
    connection.selectHostDevice('host2');
    const second = connection.call('EngineInfo', {});
    await waitFor(() => sockets[1]?.sent.length >= 2);
    staleClose();
    assert.equal(sockets[1].readyState, SocketOpen(sockets[1]));
    sockets[1].onmessage({ data: makeFrame('rpc', JSON.stringify({ id: 2, ok: { host: 2 } }) + '\n') });
    assert.equal((await second).host, 2);
  } finally { connection.disconnect(); }
});

function SocketOpen(socket) { return socket.constructor.OPEN; }

test('HTTP 401 renews the signed bearer once', async () => {
  const { connection, tailcat } = setup();
  const seen = [];
  tailcat.renew = async () => JSON.stringify({ token: 'new-token', expiresAt: Math.floor(Date.now() / 1000) + 900,
    principal: { profileId: 'profile', deviceId: 'phone' } });
  const response = { status: 200, ok: true };
  const originalFetch = global.fetch;
  try {
    global.fetch = async (_url, init) => { seen.push(init.headers.Authorization); return seen.length === 1 ? { status: 401 } : response; };
    assert.equal(await connection.request('pair/engines'), response);
    assert.deepEqual(seen, ['Bearer old-token', 'Bearer new-token']);
  } finally { global.fetch = originalFetch; connection.disconnect(); }
});
