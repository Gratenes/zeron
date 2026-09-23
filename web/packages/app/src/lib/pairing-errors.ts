import { RpcError } from "@zeron/engine-client";

/**
 * Pairing-redeem failures as user-facing sentences — the one error
 * vocabulary both pairing surfaces share (the `/pair` landing page and
 * the Settings → Devices pairing box). Moved verbatim from the deleted
 * engine drawer's `describeRedeemError` (ticket 45); the branches and
 * strings are byte-identical to the original export.
 */
export function describeRedeemError(error: unknown): string {
  if (error instanceof RpcError) {
    if (error.kind === "failed") {
      return "That pairing link did not work — it may have expired or already have been used.";
    }
    if (error.kind === "transport") {
      return "Could not reach that engine. Check the URL and that the engine is running.";
    }
    return error.message;
  }
  if (error instanceof Error && error.message.length > 0) {
    // The store's configuration-error refusal, and any storage failure.
    return error.message;
  }
  return "Pairing failed.";
}
