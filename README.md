# QuotaStation

Local-first AI usage, quota, reset, and cost monitoring for Windows.

QuotaStation is an open-source Windows application planned to bring live subscription
limits and reset windows together with historical token usage, model activity, and
API-equivalent costs across AI coding tools.

## Project status

QuotaStation is currently in the foundation and architecture phase. No installable build
is available yet.

The initial product direction is:

- Windows system tray and compact status surface
- Live quota windows and reset countdowns
- Historical input, output, and cache token usage
- Per-provider and per-model API-equivalent cost estimates
- Local-first storage with no prompt or source-code upload
- Claude, Codex, and Gemini as the first provider targets

## Principles

- **Local first:** usage data and history remain on the user's computer.
- **Read only:** provider integrations observe usage and entitlement state without changing
  accounts or subscriptions.
- **Transparent estimates:** calculated costs identify their pricing source and timestamp.
- **Extensible:** all user interfaces consume one normalized provider model.
- **Respectful reuse:** external implementations are references until license compatibility
  and attribution requirements are verified.

## Documentation

- [Architecture](docs/architecture.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Support](SUPPORT.md)

## Contributing

The implementation stack and first milestone are still being finalized. Design discussion
and focused research are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md) before opening a
pull request.

## License

QuotaStation is licensed under the [GNU Affero General Public License v3.0 only](LICENSE).
