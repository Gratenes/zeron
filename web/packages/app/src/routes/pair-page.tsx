import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  exchangeSignInCode,
  fetchSignInConfig,
  parseEngineUrl,
  type SignInConfig,
} from "@zeron/engine-client";
import { signInEngine } from "../state/fleet";
import { webDeviceLabel } from "../lib/engine-store";
import { describeSignInError } from "../lib/sign-in-errors";

type Phase = { kind: "idle" } | { kind: "connecting" } | { kind: "error"; message: string };

/**
 * The sign-in landing: enter an engine's address, then sign in — a
 * development engine takes a local user id; a WorkOS engine redirects
 * through AuthKit and the pasted sign-in code comes back here. Sign-in
 * goes through the fleet layer so a damaged configuration refuses here
 * too, and the registry starts supervising the new engine immediately.
 */
export function PairPage() {
  const [address, setAddress] = useState("");
  const [config, setConfig] = useState<SignInConfig | null>(null);
  const [userId, setUserId] = useState("");
  const [code, setCode] = useState("");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const navigate = useNavigate();

  function editAddress(value: string) {
    setAddress(value);
    setConfig(null);
  }

  async function discover(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = address.trim();
    if (phase.kind === "connecting" || trimmed.length === 0) {
      return;
    }
    setPhase({ kind: "connecting" });
    try {
      const { baseUrl } = parseEngineUrl(trimmed);
      setConfig(await fetchSignInConfig(baseUrl));
      setPhase({ kind: "idle" });
    } catch (error) {
      setPhase({ kind: "error", message: describeSignInError(error) });
    }
  }

  async function submitDev(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = userId.trim();
    if (phase.kind === "connecting" || trimmed.length === 0 || config === null) {
      return;
    }
    setPhase({ kind: "connecting" });
    try {
      const { baseUrl } = parseEngineUrl(address.trim());
      await signInEngine({
        baseUrl,
        credential: trimmed,
        label: webDeviceLabel(),
        sessionId: trimmed,
      });
      void navigate({ to: "/", replace: true });
    } catch (error) {
      setPhase({ kind: "error", message: describeSignInError(error) });
    }
  }

  async function submitCode(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = code.trim();
    if (phase.kind === "connecting" || trimmed.length === 0 || config === null) {
      return;
    }
    setPhase({ kind: "connecting" });
    try {
      const { baseUrl } = parseEngineUrl(address.trim());
      const tokens = await exchangeSignInCode(baseUrl, trimmed);
      await signInEngine({
        baseUrl,
        credential: tokens.accessToken,
        label: tokens.email ?? tokens.userId,
        sessionId: tokens.userId,
      });
      void navigate({ to: "/", replace: true });
    } catch (error) {
      setPhase({ kind: "error", message: describeSignInError(error) });
    }
  }

  return (
    <main className="pair-page">
      <h1>Connect an engine</h1>
      {phase.kind === "error" ? <p className="form-error">{phase.message}</p> : null}
      <form className="pair-form" onSubmit={discover}>
        <label className="add-engine-label" htmlFor="pair-url">
          Engine address
        </label>
        <input
          id="pair-url"
          className="input"
          type="text"
          placeholder="http://engine-host:27655"
          value={address}
          onChange={(event) => editAddress(event.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        <button
          className="btn btn-solid"
          type="submit"
          disabled={phase.kind === "connecting" || address.trim().length === 0}
        >
          {phase.kind === "connecting" ? "Connecting…" : "Continue"}
        </button>
      </form>
      {config?.mode === "dev" ? (
        <form className="pair-form" onSubmit={submitDev}>
          <label className="add-engine-label" htmlFor="pair-user">
            Development user id
          </label>
          <p className="pair-hint">
            This engine runs in development sign-in mode; the user id is the credential.
          </p>
          <input
            id="pair-user"
            className="input"
            type="text"
            placeholder="dev-user"
            value={userId}
            onChange={(event) => setUserId(event.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button
            className="btn btn-solid"
            type="submit"
            disabled={phase.kind === "connecting" || userId.trim().length === 0}
          >
            {phase.kind === "connecting" ? "Signing in…" : "Sign in"}
          </button>
        </form>
      ) : null}
      {config?.mode === "workos" ? (
        <form className="pair-form" onSubmit={submitCode}>
          <label className="add-engine-label" htmlFor="pair-code">
            Sign-in code
          </label>
          <p className="pair-hint">
            <a href={config.authorizeUrl ?? "#"} target="_blank" rel="noreferrer">
              Continue with WorkOS
            </a>
            , then paste the sign-in code the callback page shows.
          </p>
          <input
            id="pair-code"
            className="input"
            type="text"
            placeholder="state.code"
            value={code}
            onChange={(event) => setCode(event.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button
            className="btn btn-solid"
            type="submit"
            disabled={phase.kind === "connecting" || code.trim().length === 0}
          >
            {phase.kind === "connecting" ? "Signing in…" : "Complete sign-in"}
          </button>
        </form>
      ) : null}
      <p className="pair-hint">
        Enable remote access on the engine, then open this page and sign in.
      </p>
    </main>
  );
}
