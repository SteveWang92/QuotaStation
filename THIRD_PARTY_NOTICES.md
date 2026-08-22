# Third-party notices

QuotaStation includes source code from the following project.

## ccusage

- Repository: <https://github.com/ccusage/ccusage>
- Revision: `033c1f7631f603fc939fdc85163e8203f0084f83`
- Included components: `ccusage-adapter-claude`, `ccusage-adapter-codex`,
  `ccusage-adapter-common`, `ccusage-core`, `ccusage-cli`, and `ccusage-terminal`
- License: MIT
- Copyright: Copyright (c) 2025 ryoppippi
- Local use: Codex and Claude Code session discovery, parsing, replay/fork deduplication,
  aggregation, service-tier interpretation, and cost calculation
- Local modifications: each adapter exposes narrow read-only functions for its resolved usage
  directories and whether they contain session records, so QuotaStation's watcher and provider
  detection follow the exact same discovery rules as the parser; the Claude adapter also
  reports its existing daily parse grouped by local hour, so the hourly and daily views of the
  same sessions are produced by one load, one deduplication and one accumulator

The vendored subset omits upstream tests and `insta` snapshot fixtures because they are not
part of QuotaStation's dependency build. They remain available from the pinned upstream
revision.

The complete upstream MIT license is preserved at `vendor/ccusage/LICENSE`.

QuotaStation reuses ccusage's build-time pricing integration, and supplies the catalog from
the vendored snapshot described below instead of ccusage's optional downloader. The pinned
source revision is exposed in the UI for provenance. Updating the reviewed ccusage revision
also updates this price-source pin without maintaining model prices by hand.

QuotaStation does not include ccusage telemetry, credential handling, or upload behavior.

## LiteLLM model prices

- Repository: <https://github.com/BerriAI/litellm>
- Revision: `ba917681461b1ad04d30f91da26e75b3521996f3`, the revision pinned by
  `vendor/ccusage/flake.lock`
- Included component: `model_prices_and_context_window.json`
- License: MIT
- Copyright: Copyright (c) 2023 Berri AI
- Local use: the API-equivalent cost estimate, embedded at build time by ccusage's build
  script through `CCUSAGE_PRICING_JSON_PATH`
- Local modifications: entries are restricted to the model identifiers ccusage's build script
  already embeds, so the catalog compiled into the application is unchanged; every retained
  entry keeps its upstream values verbatim

Vendoring the snapshot keeps a clean build reproducible and off the network. The upstream
license is preserved at `vendor/litellm/LICENSE`; the catalog sits outside the `enterprise/`
directory that license carves out, so the MIT terms apply to it.

## Claude Code Usage Monitor

- Repository: <https://github.com/CodeZeno/Claude-Code-Usage-Monitor>
- Revision: `7b108da813550fc9500a3d8843ed207ab55b07df`
- Included component: minimal Windows taskbar parent/style/position interop adapted into
  QuotaStation's isolated taskbar adapter
- License: MIT
- Copyright: Copyright (c) 2025 Craig Constable
- Local use: locating the Windows taskbar and notification area, converting the Tauri status
  window into a non-activating taskbar child, and positioning it beside the notification area
- Local modifications: provider, credential, polling, rendering, settings, update, localization,
  and native tray implementations are excluded; QuotaStation renders its shared normalized
  snapshot through the existing Tauri React surface

The upstream MIT license text is reproduced below:

> MIT License
>
> Copyright (c) 2025 Craig Constable
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.
