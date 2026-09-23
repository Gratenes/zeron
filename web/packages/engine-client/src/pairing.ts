import type { SessionGrant } from "@zeron/proto";
import { RpcError } from "./rpc-error";

export interface RedeemOptions {
  readonly fetch?: typeof fetch;
  readonly timeoutMs?: number;
}

const DEFAULT_TIMEOUT_MS = 15_000;

/**
 * Redeem a pairing code at an engine's `POST /pairing/redeem`. The pair code
 * is the credential, so the endpoint is CORS-open on the engine side; this
 * works same-origin and cross-origin alike (ADR 0006).
 */
export async function redeemPairingCode(
  baseUrl: string,
  pairCode: string,
  label: string,
  options: RedeemOptions = {},
): Promise<SessionGrant> {
  const fetcher = options.fetch ?? fetch;
  const url = `${baseUrl.replace(/\/+$/, "")}/pairing/redeem`;
  let response: Response;
  try {
    response = await fetcher(url, {
      method: "POST",
      headers: {
        authorization: `Bearer ${pairCode}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ label }),
      redirect: "error",
      signal: AbortSignal.timeout(options.timeoutMs ?? DEFAULT_TIMEOUT_MS),
    });
  } catch {
    throw new RpcError("transport", "Could not reach the pairing endpoint");
  }
  if (response.status === 401) {
    throw new RpcError("failed", "Pairing URL was refused or has expired");
  }
  if (!response.ok) {
    throw new RpcError("transport", `Pairing endpoint returned HTTP ${response.status}`);
  }
  let grant: unknown;
  try {
    grant = await response.json();
  } catch {
    throw new RpcError("bad-reply", "Invalid pairing response");
  }
  if (
    typeof grant !== "object" ||
    grant === null ||
    typeof (grant as Record<string, unknown>).credential !== "string" ||
    typeof (grant as Record<string, unknown>).session !== "object"
  ) {
    throw new RpcError("bad-reply", "Invalid pairing response");
  }
  return grant as SessionGrant;
}

export interface ParsedPairingUrl {
  readonly baseUrl: string;
  readonly pairCode: string;
}

/**
 * Split a pairing URL (`http://host[:port]/pair#token=<code>`) the way the
 * desktop registry does: HTTP(S) only, no query or user info, `/pair` path,
 * and a `token=` fragment of URL-safe secret characters.
 */
export function parsePairingUrl(input: string): ParsedPairingUrl {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    throw new RpcError("transport", "Invalid pairing URL");
  }
  if (
    (url.protocol !== "http:" && url.protocol !== "https:") ||
    url.hostname.length === 0 ||
    url.username.length > 0 ||
    url.password.length > 0 ||
    url.search.length > 0
  ) {
    throw new RpcError("transport", "Pairing URL must use HTTP or HTTPS without a query or user information");
  }
  const token = url.hash.startsWith("#token=") ? url.hash.slice("#token=".length) : "";
  if (
    token.length === 0 ||
    ![...token].every((c) => /[A-Za-z0-9-_]/.test(c))
  ) {
    throw new RpcError("transport", "Pairing URL is missing its fragment credential");
  }
  if (!url.pathname.endsWith("/pair")) {
    throw new RpcError("transport", "Invalid pairing URL path");
  }
  const base = `${url.protocol}//${url.host}${url.pathname.slice(0, -"/pair".length)}`;
  return { baseUrl: base, pairCode: token };
}
