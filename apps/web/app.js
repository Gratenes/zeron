const encoder = new TextEncoder();
const decoder = new TextDecoder();
const INVITE_PREFIX = "kratos-pair:";
const state = { identity: null, renewal: null };

const element = id => document.getElementById(id);
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
  const output = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
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
  const generated = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  const [pkcs8, publicKey] = await Promise.all([
    crypto.subtle.exportKey("pkcs8", generated.privateKey),
    crypto.subtle.exportKey("raw", generated.publicKey),
  ]);
  const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8, "Ed25519", false, ["sign"]);
  new Uint8Array(pkcs8).fill(0);
  return { privateKey, publicKey };
}
async function post(path, body, token) {
  const response = await fetch(path, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${path}: ${await response.text() || response.status}`);
  return response.status === 204 ? null : response.json();
}
function parseInvite(value) {
  value = value.trim();
  if (value.startsWith(INVITE_PREFIX)) {
    return JSON.parse(decoder.decode(unb64url(value.slice(INVITE_PREFIX.length))));
  }
  return JSON.parse(value);
}
async function redeem() {
  const invitation = parseInvite(element("invite").value);
  if (invitation.version !== 1 || invitation.invite?.version !== 1) {
    throw new Error("Unsupported invitation version.");
  }
  const identity = await newIdentity();
  const publicKey = new Uint8Array(identity.publicKey);
  const invite = invitation.invite;
  const payload = frame("zeron.peer-auth.invite-redeem.v1", [
    Uint8Array.of(invite.version),
    invite.profileId,
    invite.inviteId,
    unb64url(invite.secret),
    publicKey,
  ]);
  const signature = await crypto.subtle.sign("Ed25519", identity.privateKey, payload);
  const principal = await post("/pair/redeem", {
    version: invite.version,
    profileId: invite.profileId,
    inviteId: invite.inviteId,
    secret: invite.secret,
    publicKey: b64url(publicKey),
    signature: b64url(signature),
    displayName: "Web browser",
  });
  state.identity = { ...identity, ...principal };
  await saveIdentity(state.identity);
}
async function authenticate() {
  const identity = state.identity;
  if (!identity) throw new Error("Pair this browser first.");
  const challenge = await post("/pair/challenge", {
    profileId: identity.profileId,
    deviceId: identity.deviceId,
  });
  const payload = frame("zeron.peer-auth.challenge.v1", [
    challenge.profileId,
    challenge.deviceId,
    challenge.challengeId,
    unb64url(challenge.nonce),
  ]);
  const signature = await crypto.subtle.sign("Ed25519", identity.privateKey, payload);
  const auth = await post("/pair/authenticate", {
    profileId: challenge.profileId,
    deviceId: challenge.deviceId,
    challengeId: challenge.challengeId,
    nonce: challenge.nonce,
    signature: b64url(signature),
  });
  window.__KRATOS_AUTH = {
    token: auth.token,
    deviceId: identity.deviceId,
  };
  element("pairing").hidden = true;
  clearTimeout(state.renewal);
  state.renewal = setTimeout(() => authenticate().catch(showError), 4 * 60 * 1000);
}
function showError(error) {
  window.__KRATOS_AUTH = null;
  element("pairing").hidden = false;
  element("error").textContent = error?.message ?? String(error);
}
window.__KRATOS_AUTH_REJECTED = () => {
  clearTimeout(state.renewal);
  showError(new Error("This browser device was revoked. Pair it again with a new invitation."));
};
async function run(action) {
  element("error").textContent = "";
  element("pair").disabled = true;
  try {
    await action();
  } catch (error) {
    showError(error);
  } finally {
    element("pair").disabled = false;
  }
}

element("pair").onclick = () => run(async () => {
  await redeem();
  await authenticate();
});
state.identity = await storedIdentity();
if (state.identity) run(authenticate);
