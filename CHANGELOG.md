# Changelog

All notable changes to QuotaStation are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each release is tagged `vX.Y.Z`
on `main`, with the matching section below as its notes.

## [Unreleased]

### Added

- Reset history now synchronizes through the shared usage folder and Codex status-line layout can be managed from Settings.

### Changed

- Quota readings are kept for 90 days instead of 14, so the quota chart reaches further back at full detail.

### Fixed

- The usage charts no longer dim and settle every time a background refresh runs behind them.
- The release build no longer exits when an enabled taskbar status widget is repositioned.
- Opening Settings no longer exits when the Codex config has no `[tui]` section.

## [0.5.0] - 2026-08-28

### Added

- An All usage range that runs from the earliest recorded usage through today.
- The reset history says how many tokens were spent inside each window that reset.
- Usage from multiple machines can be combined, inspected by device, and filtered across the history dashboard.
- The shared usage folder can be chosen with the native Windows folder picker, or typed as a path that QuotaStation offers to create.
- A "Last 24 hours" range that reads the rolling day rather than the calendar one.
- Codex reports when the first of its earned resets expires.
- The settings page lists every quota-window restart ever recorded.

### Changed

- Earned reset details now use two compact rows so the countdown and exact expiry remain readable.
- Settings is a page opened from beside Refresh rather than a dialog over the dashboard.
- The provider panels show the last restart of each window instead of the whole history.
- An early-restart note can be acknowledged, and comes back at the next restart.

### Fixed

- The quick panel no longer hides the expiry details of an earned reset.
- Conflict copies left in the shared usage folder by a sync tool no longer report the folder as failing.
- A Codex window that moves between the primary and secondary slot no longer hides the restart that came with it.

## [0.4.0] - 2026-08-23

### Added

- Ranges of up to three days are charted hour by hour instead of one column per day.
- The status line marks uncommitted files and how far the branch stands from its remote.
- A finished-turn notification names the Claude Code session as well as the project.
- A light theme, with a setting to follow Windows or pin dark or light.
- Clicking a finished-turn notification brings its terminal window back to the front.

### Changed

- The usage history opens on All rather than on the first provider.

### Fixed

- The window title bar and the scrollbars are drawn dark instead of light.
- Opening and closing the reset history quickly no longer selects the heading text.
- Sessions that finish at the same moment each raise their own notification.
- A quota window reset is no longer missed when the provider reports that window without a percentage.
- Hourly charts fall back to complete daily data until every provider has current hourly history.
- Claude history refreshes no longer parse the same session files twice for daily and hourly summaries.
- The status line counts every untracked file, resolves relative worktree pointers, and times out a slow Git status read.
- Notification clicks validate the original terminal process before bringing its window forward.

## [0.3.0] - 2026-08-21

### Added

- The dashboard charts daily tokens, cost, model mix and quota history across the selected range.
- An All tab beside the providers shows the usage history of every provider counted together.
- Every headline figure says how it moved against the period of the same length before it.
- Selecting a day in a chart or the daily table opens that day's model mix and token breakdown.
- Quota restarts are marked on the quota history chart.
- A start with Windows at logon now opens no window; a launch you perform still opens the dashboard.
- A status line setting for the project, branch, context, cache and cost detail.
- A setting for which display's taskbar carries the quota status.
- Desktop notifications for a quota window running low, a provider that cannot be read, and a quota reset.
- Diagnostics show the source commit beside the application version.

### Changed

- The application now uses a dial-and-platform icon with crisp size-specific Windows artwork.
- The status line's other-providers setting now controls the other providers and nothing else.

### Fixed

- Claude's quota survives a restart of the Claude Code CLI instead of going unavailable until the next turn.
- Provider data stays visually stable until a refresh has completed.
- The reset history's verdict column no longer wraps onto a second line.
- The application survives an Explorer restart and puts the taskbar status back by itself.
- The taskbar status no longer draws over the taskbar's application buttons.
- Claude Code's quota window restarts are now recorded and shown in the reset history.

## [0.2.0] - 2026-08-16

### Added

- Claude Code's status line now shows every provider's quota, not only Claude's own.
- Settings for the status line: how much it reports, and how providers are named in it.
- The status line names the tokens in the context window beside the share used.
- A desktop notification when Claude Code finishes a turn, offered in settings.
- The status line reports cache hits, effort, thinking, fast mode and the open pull request.
- Quota windows in the status line mark whether they are being spent faster than they elapse.
- Diagnostics name the running version and which build it is.
- Clicking the taskbar status opens the quick panel beside it.

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
- Failed or overlapping settings changes no longer corrupt, undo, or misreport the application's preferences.
- The taskbar status click target no longer intercepts drags or clicks meant for a covering window.
- The settings button no longer sits inside an empty bordered strip.
- The settings dialog is tall enough for its own contents.
- The quick panel no longer opens inside a larger window that read as a second panel behind it.

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

[Unreleased]: https://github.com/SteveWang92/QuotaStation/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/SteveWang92/QuotaStation/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/SteveWang92/QuotaStation/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/SteveWang92/QuotaStation/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/SteveWang92/QuotaStation/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/SteveWang92/QuotaStation/releases/tag/v0.1.0
