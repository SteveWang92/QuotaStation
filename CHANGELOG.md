# Changelog

All notable changes to QuotaStation are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each release is tagged `vX.Y.Z`
on `main`, with the matching section below as its notes.

## [Unreleased]

### Added

- Claude Code's status line now shows every provider's quota, not only Claude's own.
- Settings for the status line: how much it reports, and how providers are named in it.
- The status line names the tokens in the context window beside the share used.
- A desktop notification when Claude Code finishes a turn, offered in settings.
- The status line reports cache hits, effort, thinking, fast mode and the open pull request.
- Quota windows in the status line mark whether they are being spent faster than they elapse.
- Diagnostics name the running version and which build it is.

### Changed

- Every quota percentage is now the share used, not the share remaining.
- The quick panel sizes itself to its contents and grows upwards from the tray.
- Clock times are written on a 24-hour clock everywhere.
- Settings and diagnostics are one taller scrolling page instead of two tabs.
- Each quota window's source and observation time moved to the diagnostics section.
- Installing the Claude Code status line now asks in a confirmation instead of a long note.
- The status bar keeps only the settings button; each provider panel already states its own status.
- Start with Windows, the taskbar status and the desktop shortcut moved into settings.

### Removed

- Status text, provider names and a model count that each appeared twice on a surface.

### Fixed

- Each quota window is coloured by its own reading rather than the provider's loudest one.
- Claude Code's status line omits stale cross-provider quota instead of presenting it as current.
- The status line's one-row form never shows another provider's quota where Claude's own would be read.
- Removing the Claude completion hook preserves other handlers in the same Stop group.
- A failed settings write no longer changes the running application's preferences.
- The settings button no longer sits inside an empty bordered strip.
- The settings dialog is tall enough for its own contents.

## [0.1.0] - 2026-08-15

First release. QuotaStation reads the quota and usage data that the AI provider clients
already keep on this machine, and shows it without a credential, a network request, or
anything leaving the computer.

### Added

- **Codex quota windows.** The primary and secondary limits, their percentages, and their
  restart times, read through a short-lived local `app-server` process on startup and
  every five minutes.
- **Claude quota windows.** The five-hour and seven-day windows, read from Claude Code's
  session logs and — after an explicit opt-in that registers QuotaStation as Claude Code's
  status line — with the allowances Claude Code reports nowhere else. The sign-in token is
  never read; only the plan name recorded beside it.
- **Usage history.** Daily token and API-equivalent cost totals per provider, with today,
  3-, 7- and 30-day presets and a custom range, broken down by model and by input, output,
  cached input and reasoning tokens. Costs are priced from a catalog pinned at build time.
- **Quota reset history.** Codex restarts a window whenever its server decides to, not only
  when the published expiry falls due. Those restarts are recognised, classified as
  scheduled or unplanned, and kept after the samples behind them have aged out.
- **Three surfaces over one snapshot.** A tray icon with a quick panel, an optional widget
  docked into the Windows taskbar that sizes itself to the number of providers, and a
  dashboard. All three draw from the same normalized snapshot, so they can never disagree.
- **Stated data provenance.** Every quota window records which source produced it and when
  it was observed, and goes stale on a deadline appropriate to that source. A reading that
  cannot be confirmed is shown as unknown rather than as zero, and a failed read names its
  reason on the provider panel.
- **Diagnostics.** Per-acquisition-path status, session-watcher health, retention state, and
  parser and pricing catalog revisions, plus a bounded local activity log.
- **Local storage.** A SQLite database in the application data directory, holding normalized
  samples and daily summaries under a retention policy that runs at startup and every 24
  hours. Daily aggregation follows a per-provider timezone and rebuilds when it changes.

### Security

- Provider credentials, prompts, session content, and filesystem paths are redacted before
  any failure reaches the renderer, the database, or the activity log.
- All file, process, provider protocol, and database access stays in the Rust core; the
  renderer receives only normalized data over narrow commands.
- Provider integrations are read-only, and the application makes no outbound network
  requests of its own.

[Unreleased]: https://github.com/SteveWang92/QuotaStation/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SteveWang92/QuotaStation/releases/tag/v0.1.0
