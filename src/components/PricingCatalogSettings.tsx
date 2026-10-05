import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../errors";
import { formatRevision, formatTimestamp } from "../format";
import type { PricingCatalog } from "../types";

/**
 * The catalog API-equivalent costs are estimated from, and the one action that replaces it
 * with the latest LiteLLM catalog. Nothing is downloaded until the button is pressed; a newer
 * catalog then prices what the previous one could not, and leaves settled costs alone.
 */
export function PricingCatalogSettings() {
  const [catalog, setCatalog] = useState<PricingCatalog | null>(null);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<PricingCatalog>("get_pricing_catalog").then(setCatalog);
  }, []);

  const update = useCallback(async () => {
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      const before = catalog?.revision;
      const next = await invoke<PricingCatalog>("update_pricing");
      setCatalog(next);
      setOutcome(
        next.revision === before ? "Already the latest catalog." : "Updated; history repriced.",
      );
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [catalog]);

  return (
    <section className="provider-consent" aria-label="Pricing catalog settings">
      <div className="provider-consent-body">
        <h2>Pricing catalog</h2>
        <p>
          Costs are estimated from the LiteLLM catalog built into this version. Updating downloads
          the latest one from GitHub, prices models the current catalog does not know, and keeps
          every cost already calculated.
        </p>
        {catalog ? (
          <p>
            In use: <code>{formatRevision(catalog.revision)}</code>, committed{" "}
            {formatTimestamp(new Date(catalog.committedAt * 1000).toISOString())}
            {catalog.downloadedAt
              ? `, downloaded ${formatTimestamp(catalog.downloadedAt)}`
              : ", built in"}
          </p>
        ) : null}
        {error ? <p className="provider-consent-error">{error}</p> : null}
        {outcome ? <p>{outcome}</p> : null}
      </div>
      <button type="button" onClick={() => void update()} disabled={busy || catalog === null}>
        {busy ? "Updating…" : "Update pricing"}
      </button>
    </section>
  );
}
