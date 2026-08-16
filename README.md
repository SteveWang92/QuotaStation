# QuotaStation

Local-first AI usage, quota, reset, and cost monitoring for Windows.

QuotaStation is an open-source Windows application that brings live subscription
limits and reset windows together with historical token usage, model activity, and
API-equivalent costs across AI coding tools.

## Project status

QuotaStation is released and under active development. It covers Codex and Claude Code;
Gemini and other providers are future adapters. The current version and what each one
brought are in [CHANGELOG.md](CHANGELOG.md) and on the
[releases page](https://github.com/SteveWang92/QuotaStation/releases).

What it does today:

- Live quota windows, the share of each one used, and its exact restart time
- A tray icon with a quick panel, an optional widget docked into the Windows taskbar, and a
  dashboard — all three drawn from one normalized snapshot
- An optional status line for Claude Code that reports every provider's quota alongside the
  session it is running in
- Daily token and API-equivalent cost history per provider and per model, over preset and
  custom local-calendar ranges
- Quota reset history, classified as scheduled or unplanned
- Redacted acquisition, watcher, retention, and pricing diagnostics
- Local-first storage with no prompt or source-code upload, and no network request of its own

## Principles

- **Local first:** usage data and history remain on the user's computer.
- **Read only:** provider integrations observe usage and entitlement state without changing
  accounts or subscriptions.
- **Transparent estimates:** calculated costs identify their pricing source and timestamp.
- **Extensible:** all user interfaces consume one normalized provider model.
- **Respectful reuse:** prefer reviewed, compatible open-source provider and log adapters;
  pin their revisions and preserve license and attribution requirements.

## Documentation

- [Architecture](docs/architecture.md)
- [Development](docs/development.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Support](SUPPORT.md)

## Contributing

The stack is Tauri 2 with a Rust core and React/TypeScript renderer; every user interface
consumes one normalized provider model owned by the core. See
[CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## License

QuotaStation is licensed under the [GNU Affero General Public License v3.0 only](LICENSE).
