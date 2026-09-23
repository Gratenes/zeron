import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { pairEngine } from "../state/fleet";
import { webDeviceLabel } from "../lib/engine-store";
import { describeRedeemError } from "../lib/pairing-errors";

type PairPhase = { kind: "idle" } | { kind: "redeeming" } | { kind: "error"; message: string };

/**
 * The pairing landing: the engine's pairing link points here with the
 * token in the fragment (never sent to the engine as a URL part). The
 * token auto-redeems; a paste field covers manual pairing and the re-pair
 * flow after a revoked Session. Pairing goes through the fleet layer so a
 * damaged configuration refuses here too, and the registry starts
 * supervising the new engine immediately.
 */
export function PairPage() {
  const [token] = useState(() => new URLSearchParams(window.location.hash.slice(1)).get("token"));
  const started = useRef(false);
  const [phase, setPhase] = useState<PairPhase>(token !== null ? { kind: "redeeming" } : { kind: "idle" });
  const [url, setUrl] = useState("");
  const navigate = useNavigate();

  useEffect(() => {
    if (token === null || started.current) {
      return;
    }
    started.current = true;
    void (async () => {
      try {
        await pairEngine(window.location.href, webDeviceLabel());
        void navigate({ to: "/", replace: true });
      } catch (error) {
        setPhase({ kind: "error", message: describeRedeemError(error) });
      }
    })();
  }, [token, navigate]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (phase.kind === "redeeming" || url.trim().length === 0) {
      return;
    }
    setPhase({ kind: "redeeming" });
    try {
      await pairEngine(url.trim(), webDeviceLabel());
      setUrl("");
      void navigate({ to: "/", replace: true });
    } catch (error) {
      setPhase({ kind: "error", message: describeRedeemError(error) });
    }
  }

  return (
    <main className="pair-page">
      <h1>Pair this browser</h1>
      {phase.kind === "redeeming" ? <p className="pair-status">Redeeming the pairing link…</p> : null}
      {phase.kind === "error" ? <p className="form-error">{phase.message}</p> : null}
      <form className="pair-form" onSubmit={submit}>
        <label className="add-engine-label" htmlFor="pair-url">
          Pairing URL
        </label>
        <input
          id="pair-url"
          className="input"
          type="text"
          placeholder="http://engine-host:27699/pair#token=…"
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        <button className="btn btn-solid" type="submit" disabled={phase.kind === "redeeming" || url.trim().length === 0}>
          {phase.kind === "redeeming" ? "Pairing…" : "Pair engine"}
        </button>
      </form>
      <p className="pair-hint">Mint a fresh pairing link from the engine's remote access settings, then open or paste it here.</p>
    </main>
  );
}
