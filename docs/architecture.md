# Architecture

This document records the public architectural direction for QuotaStation. Detailed work
sequencing remains local until the initial implementation plan is ready to publish.

## Selected stack

- **Desktop shell:** Tauri 2
- **Application core:** Rust
- **Renderer:** React, TypeScript, and Vite
- **Persistence:** SQLite with versioned SQLx migrations owned by the Rust core
- **Initial packaging:** per-user x64 NSIS installer

The renderer receives normalized view data through narrow Tauri commands and events. It
does not read provider files, start provider processes, handle credentials, or issue SQL.

## Product boundary

QuotaStation is a local Windows application with three responsibilities:

1. Read live quota, entitlement, and reset-window state from supported local clients or
   provider endpoints.
2. Parse local usage records into normalized sessions, token categories, models, and cost
   estimates.
3. Present current state and history through a shared application core used by the system
   tray, compact monitor, and full dashboard.

Codex was the first release target. Claude Code is now a second adapter, off by default.
Gemini and other providers remain future adapters and do not block a release.

## Logical components

```text
Provider adapters ─┐
                   ├─> Normalization core ─> Local database ─> Query/service layer
Usage parsers ─────┘                                      ├─> Tray and compact monitor
Pricing catalog ──────────────────────────────────────────└─> Dashboard and history
```

### Provider adapters

Each adapter owns provider-specific discovery, authentication handoff, quota retrieval,
reset interpretation, and error mapping. Adapters return normalized data and never expose
credentials to the presentation layer.

The Codex live adapter starts the installed `codex app-server` over its JSONL stdio
transport, completes the protocol handshake, and uses only supported account, rate-limit,
and usage read operations. It listens for rate-limit updates but never logs users in or
out, changes configuration, consumes reset credits, or calls other mutation operations.

Claude Code publishes no comparable local interface: it has no usage subcommand, and no
file it writes records remaining quota or a forward-looking reset. Its only authoritative
source is Anthropic's own OAuth usage endpoint. The Claude adapter therefore reads the
access token Claude Code already stored and presents it to that endpoint, which is the
single acquisition path in the application that leaves the machine and the only one that
touches a credential. Because of that it is disabled by default and requires an explicit
in-application confirmation before it can be enabled.

The token is read in the adapter alone. It is never persisted, logged, exported in
diagnostics, or passed to the renderer. The adapter never refreshes the token itself and
never sends a chat request to read rate-limit response headers; both would write to
account state. An expired sign-in and a rate-limited read are reported as themselves, and
the endpoint's `Retry-After` is honoured so a scheduled refresh cannot hammer it.

Claude reports no reset-credit inventory, and its five-hour and seven-day windows map onto
the shared primary and secondary quota windows.

### Usage parsers

Parsers read supported local session formats and emit normalized usage records. The core
model keeps token categories separate rather than reducing them to a single total; Codex
reports input, cache read, output, and reasoning tokens.

The Codex and Claude parsers directly reuse the MIT-licensed Rust adapters from `ccusage`,
pinned to a reviewed revision and isolated behind QuotaStation's own adapter interface. Direct Cargo
consumption is preferred; minimal vendoring from the same revision is the fallback when
the upstream workspace cannot be consumed cleanly. A local parser rewrite is a last resort,
not the default.

### Normalization core

The core owns provider-neutral types for accounts, subscriptions, limits, reset windows,
usage, models, sessions, and cost estimates. User-interface surfaces must not maintain
their own competing provider models.

### Local database

SQLite is the planned store for normalized history, refresh metadata, pricing snapshots,
and schema versioning. Credentials, prompts, source content, and unnecessary raw logs do
not belong in the database.

The schema covers provider instances, current limits and samples, normalized daily usage
aggregates, refresh runs, quota rollups, and quota reset events. Event-level storage and a database-resident
pricing catalogue are not part of it: the Codex parser reports daily aggregates and
carries its own embedded pricing map. Five-minute quota samples are retained for 14 days, then hourly through day 60
and daily through day 180. Rollups preserve boundary and summary values and remain
segmented across quota resets. Successful refresh records are retained for 30 days and
failed records for 180 days, with the newest record per acquisition path always kept. Daily
usage aggregates and quota reset events are retained indefinitely. Raw
session payloads and complete local paths are never retained.

A quota reset is recorded when usage collapses to nothing, the published expiry jumps
forward, and the restarted window is anchored inside the gap between the two readings. A
window restarted more than two hours before its published expiry is classified as
unplanned. Codex writes the same rate-limit answers into its own rollout logs, so a
startup scan of those logs recovers resets that happened while QuotaStation was closed;
the scan reads only rate-limit fields, skips files older than its previous run, and never
retains conversation content. Claude Code records a reset time only inside the error it
raises once a limit has already been reached, with no usage percentage, so it has no
equivalent backfill and its restart history begins when monitoring is enabled.

## Runtime ownership

- The Rust core owns the single-instance lifecycle, tray, provider child processes,
  filesystem watching, scheduling, retries, normalization, persistence, and query services.
- The renderer owns presentation and user interaction only.
- Live Codex limits refresh on startup and every five minutes, with app-server updates
  applied immediately.
- Codex session history uses debounced filesystem changes plus a fifteen-minute full
  reconciliation.
- Normalized-data retention runs at startup when its last successful run is at least 24
  hours old. It never runs during provider refresh and does not automatically vacuum.
- Each acquisition path fails independently and preserves visibly stale last-known-good
  data.

## Open-source dependency policy

Established open-source provider and log integrations are evaluated before new parser code
is designed. Compatible implementation code may be used directly when its exact revision,
license, attribution, security behavior, and local changes are recorded. Every direct reuse
is listed in `THIRD_PARTY_NOTICES.md`; credential harvesting, browser-cookie access,
telemetry, mutation calls, and raw-data upload behavior are excluded.

## Security and privacy

- Provider access is read-only.
- Secrets remain in their originating credential store or client.
- Raw prompts, source code, and complete file paths are not collected.
- Diagnostic export must be explicit and redact account and machine identifiers.
- Network access must be attributable to a provider refresh or pricing update, and the
  Claude usage read is the only refresh that makes one.

## Deferred decisions

- Pricing catalog update policy and unknown-service-tier display behavior
- Provider plugin boundary if third-party adapters are eventually accepted
- Signing, update hosting, and distribution channel before public release
