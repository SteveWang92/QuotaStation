# QuotaStation

Local-first AI usage, quota, reset, and cost monitoring for Windows.

QuotaStation is an open-source Windows application planned to bring live subscription
limits and reset windows together with historical token usage, model activity, and
API-equivalent costs across AI coding tools.

## Project status

QuotaStation has selected its implementation architecture and is preparing the first Codex
vertical slice. No installable build is available yet.

The initial product direction is:

- Windows system tray and compact status surface
- Live quota windows and reset countdowns
- Historical input, output, and cache token usage
- Per-provider and per-model API-equivalent cost estimates
- Local-first storage with no prompt or source-code upload
- Codex as the first release target, with Claude and Gemini planned later

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
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Support](SUPPORT.md)

## Contributing

The selected stack is Tauri 2 with a Rust core and React/TypeScript renderer. The first
milestone integrates Codex live limits and local history; see
[CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## License

QuotaStation is licensed under the [GNU Affero General Public License v3.0 only](LICENSE).
