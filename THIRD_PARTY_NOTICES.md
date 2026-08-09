# Third-party notices

QuotaStation includes source code from the following project.

## ccusage

- Repository: <https://github.com/ccusage/ccusage>
- Revision: `033c1f7631f603fc939fdc85163e8203f0084f83`
- Included components: `ccusage-adapter-codex`, `ccusage-adapter-common`,
  `ccusage-core`, `ccusage-cli`, and `ccusage-terminal`
- License: MIT
- Copyright: Copyright (c) 2025 ryoppippi
- Local use: Codex session discovery, parsing, replay/fork deduplication,
  aggregation, service-tier interpretation, and cost calculation
- Local modifications: none in the included upstream Rust source

The vendored subset omits upstream tests and `insta` snapshot fixtures because they are not
part of QuotaStation's dependency build. They remain available from the pinned upstream
revision.

The complete upstream MIT license is preserved at `vendor/ccusage/LICENSE`.

QuotaStation enables ccusage's build-time pricing integration. Clean Rust builds download
the complete LiteLLM pricing catalog pinned by the vendored ccusage `flake.lock`, then
embed the GPT/OpenAI subset into the application. The pinned source revision is exposed in
the UI for provenance. Updating the reviewed ccusage revision also updates this price-source
pin without maintaining model prices by hand.

QuotaStation does not include ccusage telemetry, credential handling, or upload behavior.
