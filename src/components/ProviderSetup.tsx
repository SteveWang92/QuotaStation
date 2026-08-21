/**
 * Every surface shows the providers whose clients have written something on this machine.
 * When none have, there is nothing to display and nothing wrong with QuotaStation, so the
 * surfaces say what is missing instead of showing empty quota windows.
 */
export function ProviderSetup({ compact = false }: { compact?: boolean }) {
  return (
    <section
      className={`provider-setup${compact ? " compact" : ""}`}
      aria-label="No provider detected"
    >
      <h2>No provider client detected</h2>
      <p>
        QuotaStation reads what the provider clients leave on this machine, and found neither Codex
        nor Claude Code. Check that one is installed and signed in, send a request with it, then
        refresh.
      </p>
    </section>
  );
}
