# QuotaStation product decisions

Product direction, guardrails, and the accepted decisions behind the current design. Planned
work and its progress live in GitHub issues; released changes live in `CHANGELOG.md`.

## 1. Product direction

A production-quality, local-first Windows quota and usage monitor for AI coding
subscriptions. It answers three questions at a glance:

1. How much quota remains, and exactly when does each server-reported window reset?
2. How much local work, token usage, and API-equivalent value has accumulated over time?
3. Is the displayed state current, stale, changed unexpectedly, or unavailable?

Four surfaces — the tray quick panel, the taskbar status, Claude Code's status line, and the
dashboard — display the same normalized snapshot with no provider logic of their own. The
development focus is the Windows UI hierarchy rather than adding providers; a new provider is
added only when Steve asks and there is an approved subscription or test path to test it
against.

## 2. Authoritative data boundaries

### Live Codex quota

Read through the officially shipped Codex app-server interface: `account/read` and
`account/rateLimits/read`. The installed `codex` executable only hosts that local read
interface; QuotaStation never opens an interactive session or sends prompts, and never reads
credentials or calls private backend URLs itself.

An expired sign-in is a state, not a read failure. The app-server relays the backend's
`401 token_expired` as a JSON-RPC error on `account/rateLimits/read` while `account/read` still
answers. QuotaStation reports the provider as signed out on every surface, raises no
read-failure notification, and asks hourly instead of every five minutes. The candidate
executable that gave that reply ends the walk over the others, because they read the same
credentials.

`resetsAt` is server-owned. Display both a countdown and the exact local time; never compute a
replacement schedule from the window duration.

### Reset detection

Samples keep `used_percent`, `resets_at`, window kind and duration, and observation time, and
are kept for good (see the retention table in `docs/architecture.md`), so any questioned
restart can be diagnosed from its samples. `resets.rs` classifies a restart from consecutive observations of one window: at the
published expiry it is scheduled, materially before it unplanned, and a rolling window ageing
out earlier requests is neither. Windows are paired by duration rather than slot, because
Codex moves windows between `primary` and `secondary`.

Detection runs only on a source that publishes a server-owned percentage and expiry
(`ProviderKind::authoritative_window_source`): Codex's app-server and the quota Claude Code
hands its status line. Claude's log-derived windows are excluded, because their timing is
inferred from request times and comparing one against a published reading invents restarts.

Not classified, and added only if recorded samples show a real case: a reset time that moves
without a matching usage reset, one that moves backward or oscillates, and an unconfirmed
single anomalous sample. The recorded samples show none of these: backward moves are one or
two seconds of rounding, and forward moves without a usage drop span observation gaps where
the application was closed.

A reading with no allowance is ignored whole once its window has been measured
(`Storage::save_live`): Claude Code publishes the five-hour window with a restart but no
percentage while one closes, and storing that would discard the reading the next restart is
recognised against. A window nothing has measured is still stored, because the session logs
are the only source on a machine where Claude Code never runs beside QuotaStation. The
in-memory snapshot still carries such a reading for one refresh; nothing persists it.

A Claude restart missed live is recovered from the bridge's own record
(`claude-status-line-history.json`): the bridge runs whenever a terminal session renders,
QuotaStation open or not, and keeps the first and last reading of each run of one expiry for
35 days. The startup backfill replays that record alone, without `limit_samples`: the record
already holds every reading the samples were copied from, and the samples record no source that
would keep a log-derived window apart. A replayed restart that another device shared first
joins it through `merge_reset` as this machine's detection; a stretch with no terminal session
stays unrecoverable.

Show an anomaly only after confirmation by a fresh read or consecutive live samples, and never
repair or predict provider state in the renderer.

### Historical usage

The pinned Rust `ccusage` adapters parse local Codex and Claude sessions, deduplicate
replays and forks, normalize tokens, and price them. Historical logs cannot replace the live
server quota source.

Persist normalized facts only — never prompts, source text, tool payloads, credentials, raw
sessions, account email, or complete local paths. Quota samples convert directly to daily
summaries at the sample cutoff, with no hourly quota layer. Token usage is stored hourly for
14 days as well as daily, because a one-to-three-day range is unreadable as one column per
day; hourly rows are dropped at the cutoff rather than rolled up. Instants stay UTC; hour and
day keys follow the application zone — `time_zone` in `settings.json`, an IANA name the core
validates with `TimeZone::get`, or the Windows zone when unset — owned by `clock.rs` alone.
SQLite `'localtime'` is not used, because it can only mean the Windows zone: bucket keys and
day boundaries are computed in Rust and bound. A zone change forces a full parse, which
rebuilds that provider's rows transactionally through the `aggregation_timezone` path —
local hourly rows and daily rows from the parse's earliest day, other devices' rows by
re-import; a daily row older than the logs keeps its date rather than being dropped — and
the shared file's `timezone` is the application zone, so two machines set to one zone
exchange usage even if one's Windows zone is wrong. A wrong UTC clock is out of reach of this
setting: the clients' own log timestamps are wrong at the source.

Cache reads count towards token totals, and Claude's cache-creation tokens count as input so
the four categories add up to the parser's total. The API-equivalent cost differs from other
tools pricing the same tokens because each prices from its own catalogue; ours is the one
embedded in the pinned ccusage revision. A Claude week is almost entirely cache reads, so the
total says how much work went through the model, not how full the quota window is.

### Pricing catalog updates

A model released after a build has no price in its embedded catalog, and waiting for a
release to estimate its cost leaves weeks of sessions at zero. **Settings → Application →
Update pricing** downloads the latest LiteLLM catalog on request: one GitHub API call names
the latest commit of `model_prices_and_context_window.json`, and the file is fetched at that
commit so the revision recorded beside each cost is traceable. Nothing is downloaded on a
schedule or at startup — a background check would be a standing outbound request, which the
guardrails rule out. The download is filtered to the model prefixes ccusage embeds, kept in
the application data directory with its own format version, and used only while its commit is
newer than the embedded one, so updating the application never prices from an older download.
ccusage reads it through its own catalog-refresh hook (`set_json_fetcher`), which answers from
that file and never from the network. HTTP goes through `ureq` with the Windows TLS stack.

### Clock offset

A wrong system clock is out of reach of the zone setting, and providers publish no server
time. An opt-in `clock_check` setting, off by default, sends one SNTP request to
`time.windows.com` at startup and every two hours, carrying no user data — the only request
QuotaStation sends on a schedule. It uses `std::net::UdpSocket` (a 48-byte packet,
no new dependency) rather than parsing `w32tm`, whose output is localized, with a five-second
read timeout because UDP gives no other failure signal. `clock.rs` owns the offset and a
corrected `now()`, used by the Codex live read, the status-line windows, `Storage::save_live`
dating and restart detection, and the renderer's countdowns. A failed query keeps the previous
offset; turning the check off resets it to zero. Diagnostics shows the offset and last check,
and the dashboard warns above two minutes. Client log timestamps are not shifted.

## 3. Shared product model

```text
Provider
├─ identity and support state
├─ quota windows
│  ├─ used / remaining
│  ├─ server reset timestamp
│  ├─ observation freshness
│  └─ optional confirmed transition state
├─ reset-credit inventory
├─ daily and model usage
├─ token categories
├─ API-equivalent cost and pricing provenance
└─ acquisition diagnostics
```

Unknown values stay unknown, not zero. The Rust core owns one status vocabulary — healthy,
warning, critical, stale, unavailable — with its thresholds and copy; it sends a level, never
a colour, and React resolves the level into the active theme's token.

## 4. Surface decisions

### Quick panel

A tray left-click opens a narrow window anchored to the notification area with both quota
windows, exact reset times, reset credits, today's tokens and cost, freshness, and a link to
the dashboard. It hides on focus loss without exiting. Its height follows the renderer's
measured content, anchored at the bottom edge beside the tray. A click on the docked taskbar
widget opens the same panel beside the widget.

### Taskbar status

- A transparent, non-activating Win32 child window docked beside the notification area, using
  server-reported window durations and remaining quota.
- Explorer takes every mouse message over the taskbar, so a docked widget's webview receives no
  click; a `WH_MOUSE_LL` hook detects the click and consuming its release lets the panel take
  the foreground (`taskbar.rs::on_mouse`).
- The display is chosen by Windows device name (`taskbar_widget_display`); `Shell_TrayWnd` and
  every `Shell_SecondaryTrayWnd` are enumerated, and a detached display falls back to the
  primary taskbar. Scaling comes from `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`. Secondary
  taskbars have no `TrayNotifyWnd`, so a 160 CSS-px trailing reserve stands in for the clock.
- Explorer restarts destroy the docked child. Every window getter is guarded by `IsWindow`,
  and the widget is rebuilt under the next generation label (`taskbar-widget-N`, matched by a
  capability glob and a renderer prefix check), with an in-flight flag against duplicate
  builds.
- Docking never overlaps the centred task buttons; a taskbar too narrow for the full layout
  uses the floating fallback on the same display. Two provider slots are always reserved and
  the widget is never squeezed until a provider is clipped.
- The widget follows `SystemUsesLightTheme`, because it is drawn inside the taskbar.
- A tracker plus top-level overlay that would remove the global hook was considered and not
  taken; the docked child is accepted.

### Claude Code status line

- QuotaStation registers itself as Claude Code's status-line command and reads
  `rate_limits.five_hour` / `seven_day` from its payload — the only source for Claude's
  percentages and weekly window.
- The line reports every provider's quota plus context, cache, effort, thinking, fast mode, the
  open pull request, and the branch with uncommitted and ahead/behind counts from
  `git status --porcelain=v2 --branch` behind a three-second cache. The branch name itself
  comes from `.git/HEAD`.
- Optional segments, off by default: the rest of the payload (session name, agent, output
  style, vim mode, version, lines changed, session and API time, the 200k context flag); Git's
  stash and in-progress operation (read from files) and last commit time and tag distance (one
  `git log` behind the same cache); and QuotaStation's own counts — seven-day totals, the latest
  reset, reset credits, and the summary's age — which reach the bridge through the quota
  summary. Rows run session and project, then what Claude Code reported, then what
  QuotaStation counted.
- The layout is `statusLineLayout` in `settings.json`: segments named by string (unknown ones
  dropped, missing ones added switched off) on up to three rows, a quota format, a separator
  style, and a colour mode. Neighbouring segments of one group (model, project, request) take
  the light separator, which is what lets the default layout reproduce the fixed line exactly;
  files carrying the older switches migrate to the layout they described.
- The settings preview renders a sample session through the same `status_line()`; the renderer
  only turns its ANSI colours into theme tokens. Arbitrary commands inside the line stay out.
- Installed only from an explicit confirmation; it reports rather than replaces a foreign status
  line and removes only its own entry.
- The optional finished-turn notification names project and session title (recorded by the
  bridge, because the Stop hook receives only the session id), uses one event file per session,
  and a click raises the terminal found by walking the process tree up from the hook. Windows
  Terminal exposes no session-to-tab mapping, so only the window is raised.

### Dashboard

- The charts are one component, `TrendChart`, configured four ways over one axis: stacked
  tokens, cost, model mix, and quota history with restart markers. Ranges up to three days
  draw one column per hour.
- No charting dependency: the renderer draws its own SVG.
- Comparison is a change against the period of the same length before, stated on each headline
  figure, with direction uncoloured.
- Drill-down is a selected day from a chart column or table row.
- An All tab counts every provider in one core query; quota history is left out of it because
  percentages of different allowances do not add. An All range starts at the earliest usage
  row for the current filters.
- The chart palette is four fixed slots per theme, checked for lightness, chroma and
  colour-vision separation; a fifth series folds into "Other".
- The usage history is read two ways over one set of filters: by day, and by session. The
  sessions view is the full width of the page because it is the only place a session's two
  cost figures can be read against each other, and it replaces the day content rather than
  sitting under it. A session is placed by the local day it started on.
- Every session the parsers read is listed, whichever provider it belongs to, with a filter
  narrowing the list to the ones the client also priced. A session the client never priced
  is a row with an empty client column, not an absence: what a session cost, how long it ran
  and what it changed is the first question, and whether the client agrees is the second.
- A session view carries aggregates only — its costs, its tokens, how long it ran, how much
  code it changed, and a short form of its identifier. Project, prompt, tool-call and
  raw-session content stay out.
- Settings is a scrolling page with a section nav, and holds the complete restart history,
  with the devices that detected each restart and, on hover, each device's source, timing
  and classification. A restart more than one device saw says so in the tooltip of its
  dashboard and quick-panel marker; the layout does not change.

### Themes, notifications, diagnostics

- System, dark, or light, with dark the default. Every colour in `styles.css` is a role with a
  complete palette per theme. The core resolves the effective theme and pushes it to every
  window, and a two-second registry read publishes a system theme change, because Windows does
  not announce it to windows given an explicit theme.
- Notifications for a low-quota threshold, a failed or stale acquisition path, and a
  confirmed reset.
- Diagnostic export is a whitelist projection of the diagnostics snapshot, excluding device
  identifiers, paths, settings, history, and the activity log.
- The activity log records successes as well as failures, redacted, bounded by size alone at
  16 MB plus one roll. A line whose content matches what its shape last said is counted rather
  than written; a duration is not content, the shape is everything before the first `": "`,
  and a five-minute ceiling rewrites an unchanged line. The bridge suppresses nothing.

## 5. Multi-machine usage

Quota is account-wide and comes from the provider; token totals come from local session logs.
Normalized aggregates are exchanged through a folder a sync client already keeps in step,
because the machines are rarely online together. User-facing setup is in
`docs/multi-machine.md`. Pointing the parser at
a synced copy of another machine's session logs is rejected: it puts prompts in a sync folder.

- `hourly_usage` and `daily_usage` carry a `device` column; a `devices` table names each device
  and its last import. The device id is generated once and kept in `settings.json`, with a copy
  on the database's `local` device row that a settings file lost on its own is restored from;
  the name defaults to the computer name.
- A device whose file was left behind — a computer whose application data was wiped publishes
  under a new id — is removed by **Forget device**: its usage rows go, its restart detections
  stay, and its file is unread until its modification time moves. Nothing forgets a device
  automatically, because a machine switched off for weeks looks exactly like an abandoned one.
- A device's file is read again when it changes or when it was last read at an older
  `formatVersion` than this build reads; the Codex restart backfill watermark carries its replay
  version the same way.
- After every successful refresh that changed something, the local device's whole hourly and
  daily set, plus reset events, is written by rename to one file. Each machine writes only its
  own file and reads the others, so there is no server, primary, or merge conflict.
- The file carries numbers only. An unparseable file is skipped with a diagnostic; the usage
  rows of a file from another time zone are refused, while its restarts, being UTC instants,
  are imported; sync-client conflict copies are ignored by name.
- A restart is one account-level event and each device's detection of it an observation
  (`limit_reset_observations`), with the device and the times of the two readings compared.
  Detections of one window whose anchors fall within `min(10 minutes, duration / 10)`
  (`resets::grouping_tolerance_seconds`) are one restart: anchors come from the server's
  `resetsAt`, so devices agree to within rounding, and two real restarts of a window cannot be
  that close. `Storage::merge_reset` is the only writer — live detection, backfill and import
  all go through it — and a detection joins the nearest restart within tolerance, with no
  transitive re-clustering.
- The restart's fields come from its representative observation, chosen independent of import
  order: the narrowest bracket, then `live` before `backfill`, then the device. Other devices'
  judgements are kept and shown, never voted on. A spread above 60 seconds is shown as ±N min.
  Each window's token total is recomputed from the receiving machine's combined usage.
- Each file carries every attributed detection its machine knows, relayed ones included, so a
  restart survives its observer going offline; a detection carrying the reader's own id is
  filed as local. A file from an earlier build attributes its restarts to its own device.
- Restarts recorded before attribution are attributed once by migration 0017: a restart whose
  readings on both sides are still in `limit_samples` is local, bracketed by them, and the
  Codex rollout watermark is cleared so the next start replays every rollout through
  `merge_reset`. What is left was read in from another device and is attributed when that
  device's attributed detections arrive; until then it is "an earlier record" and is not
  exported, because a reader would credit it to the exporting machine.
- A live read that records a new restart exchanges the shared folder at once rather than
  waiting for the next history refresh.
- A machine running only Claude Code Desktop contributes usage and never quota.

## 6. Providers

Codex and Claude Code are supported. Gemini and others wait for an approved subscription or
test path. Each adapter provides server-authoritative quota separately from local history where
the provider exposes both. A provider appears on a surface only when its client has left usage
records on this machine or imported usage names it.

Claude has no local read interface for quota. Its quota comes from two sources merged per
window: the session logs as the always-on base and the status-line bridge above them (see
`docs/architecture.md`). Anthropic's OAuth usage endpoint is not used: its rate limit is shared
per account with Claude Code's own usage display, so it answers `429`. QuotaStation has no HTTP
client and reads Claude Code's sign-in file only for the plan name.

Deliberate Claude limits:

- The log-derived path reports only the five-hour window, without a percentage.
- The bridge runs only in terminal sessions; the desktop application never runs it, and the
  settings card says so when every session is desktop-hosted.
- Only the five-hour and seven-day windows are shown; the Opus, Sonnet, cowork and extra-usage
  sub-buckets are not.
- Reset backfill comes from the bridge's reading record, not the session logs: their
  limit-error reset time does not always say which bucket it belongs to.
- Cache-creation tokens count as input; Claude reports no reasoning tokens.
- Claude's live refresh runs every ten minutes and whenever its session logs change.

A provider's quota can be switched off in Settings. That covers quota alone: nothing starts its
client, no surface draws it, and its diagnostics row leaves the panel, while its history, charts
and sync carry on.

Do not adopt credential scraping, browser-cookie extraction, account switching, proxying, or
provider mutation because a reference project implements it.

## 7. Installation lifecycle

- Per-user NSIS installer (`installMode: currentUser`), no administrator rights, unsigned by
  decision; CI attaches it to each release.
- The pre-uninstall hook runs `--uninstall-cleanup`, which records the integrations actually on
  in `restore-integrations.json`, then removes only QuotaStation's status line and Stop hook.
  The next start consumes the note once. A mirror of the settings toggles is rejected: it
  cannot tell a user who turned an integration off from an uninstaller that removed it. The
  running-instance check sits inside the hook so cancelling it cannot abort an uninstall with
  the integrations already removed.
- `%APPDATA%\me.stevewang.quotastation` survives an uninstall unless **Delete app data** is
  selected.
- A `--demo` instance reads no provider on any path, including the manual refresh buttons.
- `statusline::installed()` and `notifications::installed()` match arguments, not the
  executable path, so two installed copies can take the registration from each other. Accepted:
  only a maintainer runs two copies.

## 8. Open-source reference audit

The depth-1 reference checkouts are kept outside this repository.

| Project | Audited revision | License | Approved reference boundary |
|---|---|---|---|
| OpenAI Codex | `646f7c0a91b8e327d263335da68ae8ef212895ce` | Apache-2.0 | app-server protocol and rate-limit semantics |
| ccusage | vendored pin `033c1f7631f603fc939fdc85163e8203f0084f83` | MIT | history parsers and pricing integration |
| Claude-Code-Usage-Monitor | `7b108da813550fc9500a3d8843ed207ab55b07df` | MIT | Win32 taskbar embedding, tray badges, multi-monitor and Explorer recovery |
| CodexBar-Win | `4653e98e9fe1533193e3a0a10009e90e73492696` | MIT | Windows tray-popup interaction reference only |
| juliantanx/AIUsage | `68aeeac1044191fd0c7fd24a065930b9603a789d` | MIT | dashboard and quick-panel information architecture |
| OpenUsage | `dda58e29326ac8cd63c4860197dfa0b64359a972` | MIT | normalized provider comparison and app-server parity checks |
| sylearn/AIUsage | `bbc919c084c1390d4bbef75574d73631b1484bdb` | Apache-2.0 | multi-account/menu-bar UX reference only |

Before copying any source, pin the revision, audit its dependency and data-access boundary,
preserve its license, and update `THIRD_PARTY_NOTICES.md` in the same change. Only the pinned
ccusage subset is copied into the project.

## 9. Product guardrails

- Windows-first, local-first, read-only provider access.
- Tauri 2, Rust core, React/TypeScript renderer, and Rust-owned SQLite.
- No cloud sync, telemetry, prompt upload, raw-session retention, or account mutation. The
  opt-in SNTP clock check is the only request QuotaStation sends on its own; the pricing
  update sends requests only when pressed.
- Cost is an API-equivalent estimate, never a bill.
- Provider capability absence and stale data remain explicit.
- Packaging and release are explicit user actions.
- No automatic updates: they would need a network check and a signed update chain.

## 10. Settled decisions

- Stricter TypeScript compiler options are closed; do not reopen them unless Steve asks.
- Component and `useSnapshot` rendering tests need a DOM environment and a React testing
  library the repository does not have; adding them is Steve's call.
- Code signing, CSV export, and full backup and restore are out until a user asks.
- The repository is public with one ruleset on `main` and `dev`: external changes need a pull
  request and the `Lint, test and build` check, squash merges only, no deletion or force push,
  Steve as the only bypass actor. Secret scanning, push protection, and private vulnerability
  reporting are on.
