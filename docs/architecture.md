# Architecture

QuotaStation is a local Windows application. A Rust core reads provider data, stores history
in SQLite, and sends ready-to-display results to a React interface. The interface never reads
provider files, starts provider processes, handles credentials, or queries the database.

## Technology

- **Desktop framework:** Tauri 2
- **Core:** Rust
- **Interface:** React, TypeScript, and Vite
- **Database:** SQLite with versioned SQLx migrations
- **Installer:** per-user x64 NSIS package

## Data flow

```text
Installed clients and local session files
                 │
                 ▼
        Provider-specific readers
                 │
                 ▼
          Shared Rust data model
                 │
          ┌──────┴──────┐
          ▼             ▼
       SQLite      Tauri commands
                        │
                        ▼
     Dashboard, quick panel, taskbar widget,
             and Claude Code status line
```

Every interface receives the same quota and usage data from the Rust core. This keeps
provider rules, freshness checks, status thresholds, and error handling out of React.

## Provider data

### Codex

QuotaStation starts the installed `codex app-server` as a short-lived child process and
communicates with it over JSONL on standard input and output. It uses the read-only
`account/read` and `account/rateLimits/read` operations. It does not start a login, change
Codex settings, sign the user out, or consume reset credits.

Codex usage history comes from local rollout files. The parser reads token counts, model
names, service tiers, and rate-limit snapshots without retaining conversation content.

QuotaStation contains no HTTP client and does not call OpenAI directly. The installed Codex
client remains responsible for its own authentication and any provider communication needed
to answer the local request.

### Claude Code

Claude Code sends current five-hour and seven-day quota data to a configured status-line
command. QuotaStation can register its own executable as that command after the user confirms
the change in Settings. It reads the JSON supplied by Claude Code, saves the two quota windows,
prints the configured status line, and exits.

The status-line setting belongs to Claude Code, so QuotaStation changes it only after explicit
confirmation. It will not replace a status line owned by another command, and removing the
integration deletes only the entry QuotaStation added.

Quota data is available only when Claude Code includes it in the status-line input. This
normally happens in terminal sessions after the first provider response. Claude Code Desktop
does not run the configured terminal status line, so desktop-only sessions contribute usage
history but not live quota.

Claude usage history comes from local session files. Those files can reconstruct the timing
of a five-hour session window but do not contain its allowance, so a log-only reading has no
percentage and cannot supply the seven-day window.

QuotaStation does not call Anthropic directly or read Claude Code's sign-in token. It reads
only the plan name stored beside the token.

## Usage and cost calculation

The Codex and Claude parsers reuse the MIT-licensed Rust implementation from `ccusage`, pinned
to a reviewed revision. Token categories remain separate so the interface can show input,
cached input, output, and reasoning tokens instead of one unexplained total.

Estimated API costs use the LiteLLM pricing data embedded at build time by `ccusage`. They are
comparisons, not provider bills. The application displays the pricing revision so a result can
be traced to the catalog used to calculate it.

See [Third-party notices](../THIRD_PARTY_NOTICES.md) for revisions, licenses, and local changes.

## Shared application model

The Rust core converts each provider's data into common types for providers, quota windows,
usage totals, models, costs, and errors. It also decides whether a reading is healthy, running
low, nearly gone, stale, or unavailable.

React decides how those states look in the active theme. Keeping colors out of the Rust data
allows the same reading to appear correctly in a light dashboard and a dark taskbar at the
same time. The core still chooses the effective theme because the taskbar widget must follow
the Windows taskbar rather than the application's own preference.

## Local database

SQLite stores processed usage totals, quota readings, reset history, refresh results, and the
source information needed by diagnostics. It never stores credentials, prompts, source code,
raw session records, or complete provider paths.

| Data | Retention |
| --- | --- |
| Five-minute quota readings | 90 days |
| Daily quota summaries | Indefinitely |
| Hourly usage totals | 14 days |
| Daily usage totals | Indefinitely |
| Confirmed quota reset events | Indefinitely |
| Successful refresh records | 30 days |
| Failed refresh records | 180 days |

Ranges of up to three days use hourly rows. Longer ranges use daily rows. Both are produced by
the same parse of the same local records, so changing the selected range does not re-read the
provider files.

Daily rows follow the current Windows time zone. If that time zone changes, the next complete
parse replaces the affected provider's daily rows in one transaction so dates from two zones
are not mixed.

Quota readings are summarized by the highest percentage observed during each day. This
preserves a window that filled and reset before the last reading of the day.

Retention runs at startup and once every 24 hours while the application remains open. It does
not run during a provider refresh and does not automatically compact the database with
`VACUUM`.

## Quota reset history

A reset is recorded when consecutive server-supplied readings show that usage dropped, the
expiry moved forward, and the new window began between those readings. A reset more than two
hours before the previous expiry is shown as a possible early reset rather than as a
provider-confirmed fact.

Only a source that supplies both a percentage and an expiry can prove a reset. This includes
Codex app-server readings and Claude Code status-line readings. A window inferred only from
session timestamps cannot create a reset event.

Codex also writes rate-limit snapshots to its rollout logs. QuotaStation can use those fields
to recover resets that happened while it was closed without retaining conversation content.
Claude Code has no equivalent source, so its reset history begins when monitoring is enabled.

Each reset event keeps an estimated token total for the window that ended. Hourly usage is
credited to the window active at the start of that hour, so the estimate can be imprecise at
the two boundary hours. The interface marks it with a tilde for that reason.

## Multi-machine usage

Each computer can export its own hourly and daily totals to a shared folder and import totals
written by the others. The files contain no prompts, paths, sessions, credentials, or account
details. See [Multi-machine usage](multi-machine.md) for the file contents and setup.

## Runtime responsibilities

- The Rust core manages the single running instance, system tray, child processes, file
  watching, scheduled refreshes, storage, and diagnostics.
- Codex quota refreshes at startup and every five minutes through a short-lived app-server
  process.
- Claude quota refreshes when Claude Code supplies a status-line reading or its session files
  change.
- Session-file watchers are reconciled every fifteen minutes so a missed Windows notification
  does not leave history stale indefinitely.
- Each provider data source fails independently. A failed source keeps its last successful
  result visible but marks it stale.
- A provider that reports an expired sign-in is shown as signed out rather than failed, and
  its quota is read once an hour until someone signs in with that client again.
- A provider's quota can be switched off in Settings. Nothing then starts its client to
  read a percentage and no surface draws one. Its usage history is unaffected: the session
  files are parsed and watched as before, and the provider keeps its place in the charts.
- A normal launch opens the dashboard. `--background` starts in the tray, which is how the
  Windows logon entry runs it.
- A second launch hands control to the existing process instead of opening another database
  connection.
- The taskbar widget reserves complete provider slots. If the selected taskbar is too narrow,
  the widget floats beside it instead of clipping a provider.

## Security and privacy

- Provider access is read-only.
- Credentials stay in the provider client or operating-system credential store.
- Prompts, source code, raw sessions, account details, and complete paths are not collected.
- Diagnostic export is an explicit user action and omits account and machine identifiers.
- The activity log records what the application did — reads, publications, queries, window
  and settings changes, renderer failures — but no session content, credential, or provider
  path. It is bounded by size alone: 16 MB, then one roll.
- The pricing catalog is embedded at build time, so a clean build does not download it.

## Reused code

Existing open-source provider and log readers are reviewed before new parser code is written.
Reused code must have a pinned revision, a compatible license, and a record of its security
behavior and local changes in [Third-party notices](../THIRD_PARTY_NOTICES.md). Code that reads
browser cookies, uploads raw data, collects telemetry, or changes provider accounts is not
included.
