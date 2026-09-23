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

Every session the parsers read is stored on its own as well as in the day it belongs to:
its cost from the catalog, its tokens, its models, and the span from its first entry to its
last. Recent Claude Code sessions also carry the client's own cost accounting in their logs,
and that figure is stored beside the computed one, so the pricing catalog can be checked
against the vendor's own numbers instead of being taken on trust. Neither figure is a bill,
and only some sessions can be compared at all: Claude Code began recording its figure
partway through its life and Codex records none, so a session without one carries the
catalog's estimate alone. A session whose entries already carry a cost of their own is
marked as no longer independently priced, because the parser then reports the client's
number back rather than a second opinion. The client's record also says how much of the
session was spent waiting on the provider and how much code it changed, which the sessions
view lists beside the costs.

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
raw session records, or complete provider paths. The one identifier it keeps is the client's
own session id on a stored cost comparison, which is what lets a growing session correct its
own row; it stays in the local database and is not part of the shared-folder export.

| Data | Retention |
| --- | --- |
| Five-minute quota readings | Indefinitely |
| Daily quota summaries | Indefinitely |
| Hourly usage totals | 14 days |
| Daily usage totals | Indefinitely |
| Confirmed quota reset events | Indefinitely |
| Per-session summaries | Indefinitely |
| Successful refresh records | 30 days |
| Failed refresh records | 180 days |

Everything that records usage or quota is kept for good. It is the only copy once the
providers' logs are gone, and together it grows by tens of megabytes a year. Two things are
dropped on purpose: hourly usage past the fourteen days the parser fills, because a zone change
rebuilds it from the logs and an hour older than them could not be rebuilt, and refresh
records, which only Diagnostics reads.

Ranges of up to three days use hourly rows. Longer ranges use daily rows. Both are produced by
the same parse of the same local records, so changing the selected range does not re-read the
provider files.

Instants are stored in UTC. Only the hour and day a piece of usage is filed under, and every
time a surface shows, depend on a time zone, and both follow the application zone: the one
chosen in **Settings → General**, or the Windows zone when none is chosen. The core computes
every bucket key and day boundary itself rather than asking SQLite, whose local time can only
mean the Windows zone. When the zone changes, the next complete parse rebuilds the affected
provider's hourly rows and every daily row the session logs still reach, in one transaction,
and rows imported from other devices are read again, so hours from two zones are never mixed.
A day older than the logs — Claude Code deletes old transcripts — cannot be rebuilt and keeps
the date it was filed under rather than being discarded. It also keeps the cost it was priced
at; its per-model token counts are still there if it ever has to be priced again.

The zone setting corrects how times are displayed and bucketed, not the clock itself. If the
computer's clock is wrong in UTC, the timestamps Codex and Claude Code write into their logs
are wrong at the source, and no display zone can repair them.

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
- A session identifier is stored only so a session's row can be corrected as it grows, and
  it is never exported or shared.
- Diagnostic export is an explicit user action and omits account and machine identifiers.
- The activity log records what the application did — reads, publications, queries, window
  and settings changes, renderer failures — but no session content, credential, or provider
  path. It is bounded by size alone: 16 MB, then one roll.
- The pricing catalog is embedded at build time, so a clean build does not download it.
- The only request QuotaStation sends of its own is the opt-in clock check below, which
  carries no user data.

## Clock check

A wrong computer clock breaks what a time zone cannot fix: countdowns run against it, and a
reading dated by it no longer lines up with the reset times the server publishes. Providers
publish no server time, so the clock can be checked against internet time instead. The check
is off unless it is switched on in **Settings → General**. When on, the core sends one SNTP
request to `time.windows.com` at startup and every two hours — QuotaStation's only outbound
request of its own, a 48-byte packet carrying nothing but the computer's current time — and
applies the measured offset wherever a reading is dated or compared with server time, and to
the interface's countdowns. A failed check keeps the previous offset and is reported in
Diagnostics; switching the check off forgets the offset. Timestamps the clients wrote into
their own logs are left as written, because the offset when they were written is unknown.
Stored readings and restarts do not record whether an offset applied when they were dated, so
they are never re-dated afterwards.

## Changing stored or shared data

A record that outlives its source can never be corrected by re-reading that source, and a
shared file is read by builds older and newer than the one that wrote it. A change to what is
stored or shared therefore answers these before it lands:

- **Instants are UTC.** A local day or hour is derived from an instant when it is read. A table
  that has to store a local key records the zone it was keyed in, as `aggregation_timezone`
  does for usage, so a zone change can find what it has to rebuild.
- **What cannot be rebuilt is named.** A row that survives its source log records what produced
  it — parser revision, pricing catalog, zone — or the fact that it can never be recomputed
  is written here, so a later fix knows which rows it cannot reach.
- **Shared records carry their origin.** Every record a device exports says which device
  produced it. A reader never infers origin from whose file a record arrived in, and a record
  of unknown origin is not exported.
- **The shared format has a version.** `FORMAT_VERSION` in `sync.rs` moves whenever a field
  is added whose absence an older file cannot be told apart from, or whose meaning changes. A
  reader accepts every version up to its own and reports a newer one as needing an update.
- **Mixed versions are expected.** Two computers can run different builds while both are on,
  and either can upgrade first; each change is checked for what an older reader makes of the
  new file and what the new reader makes of an old one.
- **Migrations are forward-only.** An older build refuses a database a newer one has
  migrated, so a migration that removes or rewrites data is one there is no stepping back
  from.

## Reused code

Existing open-source provider and log readers are reviewed before new parser code is written.
Reused code must have a pinned revision, a compatible license, and a record of its security
behavior and local changes in [Third-party notices](../THIRD_PARTY_NOTICES.md). Code that reads
browser cookies, uploads raw data, collects telemetry, or changes provider accounts is not
included.
