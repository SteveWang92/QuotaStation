# Architecture

This document records the public architectural direction for QuotaStation. Detailed work
sequencing remains local until the initial implementation plan is ready to publish.

## Product boundary

QuotaStation is a local Windows application with three responsibilities:

1. Read live quota, entitlement, and reset-window state from supported local clients or
   provider endpoints.
2. Parse local usage records into normalized sessions, token categories, models, and cost
   estimates.
3. Present current state and history through a shared application core used by the system
   tray, compact monitor, and full dashboard.

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

### Usage parsers

Parsers read supported local session formats and emit normalized usage events. The core
model distinguishes input, output, cache read, and cache write tokens rather than reducing
them to a single total.

### Normalization core

The core owns provider-neutral types for accounts, subscriptions, limits, reset windows,
usage, models, sessions, and cost estimates. User-interface surfaces must not maintain
their own competing provider models.

### Local database

SQLite is the planned store for normalized history, refresh metadata, pricing snapshots,
and schema versioning. Credentials, prompts, source content, and unnecessary raw logs do
not belong in the database.

## Security and privacy

- Provider access is read-only.
- Secrets remain in their originating credential store or client.
- Raw prompts, source code, and complete file paths are not collected.
- Diagnostic export must be explicit and redact account and machine identifiers.
- Network access must be attributable to a provider refresh or pricing update.

## Open decisions

- Native Windows application stack and packaging format
- Background refresh ownership and process lifecycle
- Provider plugin boundary for future third-party integrations
- Pricing catalog source, update policy, and offline behavior
- Retention and aggregation policy for detailed usage events
