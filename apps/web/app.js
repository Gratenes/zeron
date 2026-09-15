const $ = id => document.getElementById(id);
const encoder = new TextEncoder();
const decoder = new TextDecoder();
const INVITE_PREFIX = "kratos-pair:";
const state = { identity: null, token: null, socket: null, nextId: 1, pending: new Map() };

const b64url = bytes => {
  let value = "";
  for (const byte of new Uint8Array(bytes)) value += String.fromCharCode(byte);
  return btoa(value).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
};
const unb64url = value => {
  value = value.replaceAll("-", "+").replaceAll("_", "/");
  const raw = atob(value + "=".repeat((4 - value.length % 4) % 4));
  return Uint8Array.from(raw, char => char.charCodeAt(0));
};
const frame = (domain, fields) => {
  const parts = [encoder.encode(domain + "\0")];
  for (const field of fields) {
    const bytes = typeof field === "string" ? encoder.encode(field) : new Uint8Array(field);
    const size = new Uint8Array(4);
    new DataView(size.buffer).setUint32(0, bytes.length);
    parts.push(size, bytes);
  }
  const length = parts.reduce((sum, part) => sum + part.length, 0);
  const output = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) { output.set(part, offset); offset += part.length; }
  return output;
};

function openIdentityDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open("kratos-browser-identity", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("identity");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}
async function storedIdentity() {
  const db = await openIdentityDb();
  return new Promise((resolve, reject) => {
    const request = db.transaction("identity").objectStore("identity").get("device");
    request.onsuccess = () => resolve(request.result ?? null);
    request.onerror = () => reject(request.error);
  });
}
async function saveIdentity(identity) {
  const db = await openIdentityDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction("identity", "readwrite");
    tx.objectStore("identity").put(identity, "device");
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
  });
}
async function newIdentity() {
  if (!crypto.subtle) throw new Error("WebCrypto is unavailable; use HTTPS or localhost.");
  const pair = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  const [pkcs8, publicKey] = await Promise.all([
    crypto.subtle.exportKey("pkcs8", pair.privateKey),
    crypto.subtle.exportKey("raw", pair.publicKey),
  ]);
  const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8, "Ed25519", false, ["sign"]);
  new Uint8Array(pkcs8).fill(0);
  return { privateKey, publicKey };
}
async function post(path, body, token) {
  const response = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${path}: ${await response.text() || response.status}`);
  return response.status === 204 ? null : response.json();
}
async function get(path, token) {
  const response = await fetch(path, { headers: { authorization: `Bearer ${token}` } });
  if (!response.ok) throw new Error(`${path}: ${await response.text() || response.status}`);
  return response.json();
}
function parseInvite(value) {
  value = value.trim();
  if (value.startsWith(INVITE_PREFIX)) {
    return JSON.parse(decoder.decode(unb64url(value.slice(INVITE_PREFIX.length))));
  }
  return JSON.parse(value);
}
async function pair() {
  const invitation = parseInvite($("invite").value);
  if (invitation.version !== 1 || invitation.invite?.version !== 1) throw new Error("Unsupported invitation version.");
  const identity = await newIdentity();
  const publicKey = new Uint8Array(identity.publicKey);
  const invite = invitation.invite;
  const payload = frame("zeron.peer-auth.invite-redeem.v1", [
    Uint8Array.of(invite.version), invite.profileId, invite.inviteId, unb64url(invite.secret), publicKey,
  ]);
  const signature = await crypto.subtle.sign("Ed25519", identity.privateKey, payload);
  const principal = await post("/pair/redeem", {
    version: invite.version,
    profileId: invite.profileId,
    inviteId: invite.inviteId,
    secret: invite.secret,
    publicKey: b64url(publicKey),
    signature: b64url(signature),
    displayName: $("device-name").value.trim() || null,
  });
  state.identity = { ...identity, ...principal };
  await saveIdentity(state.identity);
}
async function authenticate() {
  if (!state.identity) throw new Error("Pair this browser first.");
  const identity = state.identity;
  const challenge = await post("/pair/challenge", {
    profileId: identity.profileId,
    deviceId: identity.deviceId,
  });
  const payload = frame("zeron.peer-auth.challenge.v1", [
    challenge.profileId, challenge.deviceId, challenge.challengeId, unb64url(challenge.nonce),
  ]);
  const signature = await crypto.subtle.sign("Ed25519", identity.privateKey, payload);
  const auth = await post("/pair/authenticate", {
    profileId: challenge.profileId,
    deviceId: challenge.deviceId,
    challengeId: challenge.challengeId,
    nonce: challenge.nonce,
    signature: b64url(signature),
  });
  state.token = auth.token;
  $("pair-card").hidden = true;
  $("probe-card").hidden = false;
  $("identity").textContent = `Browser device ${identity.deviceId}`;
}

function encodeDeviceFrame(payload) {
  const header = encoder.encode(JSON.stringify({ s: "rpc", k: "rpc" }));
  const prefix = [];
  for (let n = header.length; ; n >>= 7) {
    prefix.push((n & 0x7f) | (n > 0x7f ? 0x80 : 0));
    if (n <= 0x7f) break;
  }
  const bytes = new Uint8Array(prefix.length + header.length + payload.length);
  bytes.set(prefix); bytes.set(header, prefix.length); bytes.set(payload, prefix.length + header.length);
  return bytes;
}
function decodeDeviceFrame(value) {
  const bytes = new Uint8Array(value);
  let length = 0, shift = 0, offset = 0, byte;
  do { byte = bytes[offset++]; length |= (byte & 0x7f) << shift; shift += 7; } while (byte & 0x80);
  const header = JSON.parse(decoder.decode(bytes.slice(offset, offset + length)));
  return { header, payload: bytes.slice(offset + length) };
}
async function connect() {
  await authenticate();
  const devices = await get("/pair/engines", state.token);
  const engine = devices.find(device => device.owner && device.revokedAt == null);
  if (!engine) throw new Error("No active owner engine device found.");
  state.socket?.close();
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${scheme}//${location.host}/device/${encodeURIComponent(engine.deviceId)}/ws`, [
    "kratos-rpc", `kratos-bearer.${state.token}`,
  ]);
  socket.binaryType = "arraybuffer";
  socket.onmessage = event => {
    if (typeof event.data === "string") return;
    const { header, payload } = decodeDeviceFrame(event.data);
    if (header.k !== "rpc") return;
    const response = JSON.parse(decoder.decode(payload));
    const pending = state.pending.get(response.id);
    if (!pending) return;
    state.pending.delete(response.id);
    response.err ? pending.reject(new Error(response.err)) : pending.resolve(response.ok);
  };
  socket.onclose = () => {
    for (const pending of state.pending.values()) pending.reject(new Error("Relay disconnected or device revoked"));
    state.pending.clear();
    setConnected(false, "Disconnected or revoked");
  };
  socket.onerror = () => setConnected(false, "Relay connection failed");
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    const timer = setTimeout(() => reject(new Error("Relay connection timed out.")), 10000);
    socket.addEventListener("open", () => clearTimeout(timer), { once: true });
  });
  state.socket = socket;
  setConnected(true, `Connected to ${engine.displayName || engine.deviceId}`);
  $("engine").textContent = JSON.stringify(await rpc("ListHarnesses", null), null, 2);
}
function rpc(method, params) {
  const id = state.nextId++;
  const payload = encoder.encode(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { state.pending.delete(id); reject(new Error("RPC timeout")); }, 10000);
    state.pending.set(id, {
      resolve: value => { clearTimeout(timeout); resolve(value); },
      reject: error => { clearTimeout(timeout); reject(error); },
    });
    state.socket.send(encodeDeviceFrame(payload));
  });
}
async function measure() {
  const samples = [];
  $("measure").disabled = true;
  for (let i = 0; i < 25; i++) {
    const started = performance.now();
    await rpc("ListHarnesses", null);
    const elapsed = performance.now() - started;
    samples.push(elapsed);
    $("latest").textContent = `${elapsed.toFixed(1)} ms`;
  }
  samples.sort((a, b) => a - b);
  $("median").textContent = `${samples[12].toFixed(1)} ms`;
  $("p95").textContent = `${samples[23].toFixed(1)} ms`;
  $("range").textContent = `${samples[0].toFixed(1)}–${samples[24].toFixed(1)} ms`;
  $("measure").disabled = false;
}
function setConnected(connected, message) {
  $("dot").classList.toggle("live", connected);
  $("status").textContent = message;
  $("measure").disabled = !connected;
}
async function action(fn) {
  $("error").textContent = "";
  try { await fn(); } catch (error) { $("error").textContent = error.message; }
}

$("pair").onclick = () => action(async () => { await pair(); await connect(); });
$("connect").onclick = () => action(connect);
$("measure").onclick = () => action(measure);
state.identity = await storedIdentity();
if (state.identity) action(connect);
