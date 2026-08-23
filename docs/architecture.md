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

Codex was the first release target. Claude Code is now a second adapter. Both are shown
whenever their client has left usage records on this machine, and a provider that has left
none is not shown at all. Gemini and other providers remain future adapters and do not
block a release.

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
transport, completes the protocol handshake, and uses only the supported `account/read` and
`account/rateLimits/read` operations. It never logs users in or out, changes configuration,
consumes reset credits, or calls other mutation operations.

Claude Code publishes no comparable local interface: it has no usage subcommand, and no
file it writes on its own records remaining quota or a forward-looking reset. It does,
however, report its quota to one place. Since Claude Code 2.1.80 the JSON it hands the
command configured as its status line carries `rate_limits.five_hour` and
`rate_limits.seven_day`, each with the percentage consumed and the epoch second the window
restarts — the same pair of windows Anthropic's usage endpoint reports, delivered locally.
QuotaStation therefore offers to register itself as that command. The registered process is
this same executable started with `--claude-statusline`: it reads the payload, stores the
two windows in the application data directory, prints a status line, and exits without ever
reaching the interface. No credential is read, nothing leaves the machine, and no rate limit
is shared with Claude Code's own usage display.

That bridge changes a setting inside Claude Code's own configuration, so it is only ever
installed from an explicit action in the dashboard's settings dialog, a status line belonging
to something else is reported rather than replaced, and removing it takes out only the entry
QuotaStation wrote. Its readings arrive only from terminal sessions: a status line is
something a terminal renders, and Claude Code hosted inside the desktop application draws its
own interface and never runs the configured command. QuotaStation therefore reads the entry
point of each live session Claude Code records, so an installation whose sessions are all
desktop-hosted can say why no reading arrives instead of looking broken. That limit is also
why the session logs stay underneath the bridge: they are written whatever the host. Claude Code's
session window is a rolling five hours opened by the first request after the previous window
closed, so the log adapter recovers that window's timing from request times alone, and a
limit the account actually reached is stated exactly in the error Claude raises, which
outranks the inferred one. What the logs never carry is the allowance, so on that path alone
the percentage consumed stays unknown and the seven-day window is not visible at all.

Anthropic's OAuth usage endpoint was a third source and is not one any more. It reports the
same two windows, but it rate-limits an account as a whole and Claude Code's own usage
display is already spending that budget, so in practice it answered `429` and nothing else —
a source that costs a stored credential and never succeeds is worse than no source. Claude
Code's sign-in is now read for one field, the plan name recorded beside the token, which
needs no request and never leaves the machine. QuotaStation makes no network request for
Claude at all.

The two remaining sources are combined by window rather than by precedence over the whole
reading: the better-informed source owns a window both describe, and a window only one of
them knows about is kept. A source that cannot answer therefore never blanks a window the
other one already filled.

Claude reports no reset-credit inventory. Its five-hour and seven-day windows map onto the
shared primary and secondary quota windows; the log-derived path fills the primary one
only, and without a percentage.

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

The line stops at appearance. The core decides how loud a reading is — healthy, warning,
critical, on thresholds every surface shares — and never what colour that is: the same 95%
is one red on a near-black dashboard and another on a white one, and a snapshot the core
had already coloured could not be drawn correctly in both. The renderer resolves a level
into a theme token, which is also why one snapshot can serve a light window and a dark one
at the same moment.

Which theme each surface is in is the core's answer, though, not the renderer's. A WebView2
reports `prefers-color-scheme` from the theme the application has just set on its window,
so a renderer asked to work it out would only hear its own echo; the core reads what Windows
is actually set to. It publishes two answers, because the taskbar widget is drawn inside the
Windows taskbar and has to match the taskbar rather than the user's preference for
QuotaStation's own windows.

### Local database

SQLite is the planned store for normalized history, refresh metadata, pricing snapshots,
and schema versioning. Credentials, prompts, source content, and unnecessary raw logs do
not belong in the database.

Daily usage is bucketed with the Windows system IANA time zone recorded for each provider.
When that zone changes, the next successful full log parse replaces that provider's daily
rows transactionally so dates from the old and new zones cannot be mixed. Existing databases
adopt their current zone without a destructive first-run rebuild.

The schema covers provider instances, current limits and samples, normalized daily usage
aggregates, the hourly usage of the recent window, refresh runs, quota rollups, and quota
reset events. Event-level storage and a database-resident
pricing catalogue are not part of it: the Codex parser reports daily aggregates and
carries its own embedded pricing map. Five-minute quota samples are retained for 14 days,
then converted directly into daily summaries retained indefinitely. Usage is stored a second
time at hourly resolution, because a range of a day or three says nothing as one column per
day; those rows are retained for 14 days and then dropped rather than rolled up, since the
daily rows already hold everything they summarise. Rollups preserve
boundary and summary values and remain segmented across quota resets. Successful refresh records are retained for 30 days and
failed records for 180 days, with the newest record per acquisition path always kept. Daily
usage aggregates and quota reset events are retained indefinitely. Raw
session payloads and complete local paths are never retained.

A range of up to three days is read hour by hour and anything longer day by day. Both
resolutions come from one parse of the session logs: an hour is aggregated by the same
adapter code the day containing it is, so the two can never disagree about the same usage.
Quota history stays daily whatever the usage beside it is read at — a quota reading is a
poll rather than a request, and a day is described by the fullest that window got on it.

The usage history read takes an optional provider: naming one answers for that provider,
naming none counts every provider instance together. The combined answer is one query with
the filter dropped rather than separate reads added up in the renderer, so its totals, its
per-day stack and its model ranking are built exactly as a single provider's are. Quota has
no combined form — one provider's window says nothing about another's allowance — so that
chart belongs to a named provider only.

Quota history over a date range is answered from both stores at once, and they do not
overlap: the readings inside the retention window come from the samples at the granularity
they arrived at, and everything older from the daily rollups. Each day is reduced to the
fullest that window got on it rather than to its last reading, so a window that filled and
restarted inside one day still reports how full it got; the restart itself is carried by the
reset events beside it.

A Codex app-server quota reset is inferred when usage falls materially, the published expiry
jumps forward, and the restarted window is anchored inside the gap between the two readings. A
window that appears to have restarted more than two hours before its published expiry is
classified internally as unplanned. This is a heuristic derived from adjacent samples, so
the interface labels it as a possible early reset rather than provider-confirmed fact.

It is applied per source rather than per provider: only readings from the one source that
publishes a window — Codex's app-server, and the quota Claude Code hands its status line —
can evidence a restart of it. A window recovered from local session logs is derived from
request times instead, and comparing one of those against a published reading, or against
the next derived guess, would manufacture restarts that never happened.
Codex writes the same rate-limit answers into its own rollout logs, so a
startup scan of those logs recovers resets that happened while QuotaStation was closed;
the scan reads only rate-limit fields, skips files older than its previous run, and never
retains conversation content. Some Claude Code log formats may record a reset time inside
an error raised once a limit has already been reached, but the normalized entry does not
identify its quota bucket. QuotaStation therefore does not attach that ambiguous timestamp
to the five-hour window. Claude has no equivalent reset backfill, and its restart history
begins when monitoring is enabled.

Each recorded restart also carries the tokens the window it closed spent. That window runs
from the restart before it to the earlier of its published expiry and this restart: a window
rebuilt early stops when the restart anchored the next one, while one that expired unused
stops at its expiry and is followed by however long it took for the next request to anchor
the next window — an idle gap that belongs to neither. The span is held to the window's own
length as well, so a restart nothing recorded between cannot credit one window with days of
work; a window that restarted early is shorter than its length rather than longer, so that
limit never applies to one of those. An hour is credited to whichever window was running
when it opened, so no hour is counted twice; the figure is therefore approximate at the two
boundaries and exact between them, which is why the interface writes it with a tilde.

Hourly rows are kept for 14 days and reset events indefinitely, so the total is stored on
the event rather than summed on the way out. It is rebuilt on every write for as long as all
the hours behind it are still stored, and left alone once the oldest of them is pruned: a
restart from last year keeps the figure it was given at the time, and one older than the
hourly window has ever covered reports nothing rather than a zero.

## Runtime ownership

- The Rust core owns the single-instance lifecycle, tray, provider child processes,
  filesystem watching, scheduling, retries, normalization, persistence, and query services.
- Whether a start opens a window is the core's decision, and it is made from the launch
  itself: the logon entry Windows runs is registered with `--background` and comes up in
  the tray alone, while a launch a person performs opens the dashboard. Either kind reaching
  an already-running instance hands over to it rather than starting a second one, and a
  hand-over from a launch carrying no argument is what reopens a dashboard closed to the tray.
- The Rust taskbar adapter owns a whole-slot width contract with two provider slots reserved.
  Additional providers grow it by a complete slot; if Explorer cannot supply the full width,
  the widget uses its floating fallback instead of clipping normalized provider data. Docked
  height follows Explorer's physical taskbar height, preserving the same logical room at high DPI.
- The renderer owns presentation and user interaction only.
- Live Codex limits refresh on startup and every five minutes through a short-lived
  app-server process.
- Codex session history uses debounced filesystem changes plus a fifteen-minute full
  reconciliation. The watcher rechecks provider roots every minute, so a client installed
  or first used after QuotaStation starts becomes watched without restarting the app. An
  operating-system watcher error keeps diagnostics degraded until the failed watcher has
  been rebuilt and every currently expected location is watched again.
- Normalized-data retention runs at startup, and every 24 hours thereafter, when its last
  successful run is at least 24 hours old; the periodic pass keeps a process that stays
  resident for weeks from accumulating samples indefinitely. It never runs during provider
  refresh and does not automatically vacuum.
- Each acquisition path fails independently and preserves visibly stale last-known-good
  data.
- History refresh samples at most the final 256 KiB of each of the 16 most recently modified
  JSONL files for structural quality, bounding the additional read to roughly 4 MiB.
  Three or more candidate records with no compatible records, or at least 25 percent
  incompatible candidates, report schema incompatibility and preserve last-known-good data;
  isolated malformed lines remain tolerated.

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
- No acquisition path makes a network request. Both providers are read from local clients
  and local files, and the pricing catalog is embedded at build time from a pinned snapshot.
- The application and the status-line bridge append an activity log to the application data
  directory, recording which source answered and why a read failed. It carries no session
  content, no credential, and no provider paths, and is rolled over at half a megabyte.

## Deferred decisions

- Pricing catalog update policy and unknown-service-tier display behavior
- Provider plugin boundary if third-party adapters are eventually accepted
- Signing, update hosting, and distribution channel before public release
