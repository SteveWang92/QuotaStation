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

Codex is the first release target. Claude, Gemini, and other providers remain future
adapters and do not block a useful Codex-only release.

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

### Usage parsers

Parsers read supported local session formats and emit normalized usage events. The core
model distinguishes input, output, cache read, and cache write tokens rather than reducing
them to a single total.

The Codex parser directly reuses the MIT-licensed Rust adapter from `ccusage`, pinned to a
reviewed revision and isolated behind QuotaStation's own adapter interface. Direct Cargo
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

The initial schema covers provider instances, current limits and samples, normalized usage
events and daily aggregates, pricing entries, ingestion cursors, and refresh runs. Raw
session payloads and complete local paths are never retained.

## Runtime ownership

- The Rust core owns the single-instance lifecycle, tray, provider child processes,
  filesystem watching, scheduling, retries, normalization, persistence, and query services.
- The renderer owns presentation and user interaction only.
- Live Codex limits refresh on startup and every five minutes, with app-server updates
  applied immediately.
- Codex session history uses debounced filesystem changes plus a fifteen-minute full
  reconciliation.
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
- Network access must be attributable to a provider refresh or pricing update.

## Deferred decisions

- Exact pinned `ccusage` revision and Cargo-versus-vendored integration form
- Pricing catalog update policy and unknown-service-tier display behavior
- Detailed-event retention period after real data volume is known
- Provider plugin boundary if third-party adapters are eventually accepted
- Signing, update hosting, and distribution channel before public release
