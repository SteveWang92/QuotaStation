# Development

QuotaStation is a Windows-first Tauri 2 application with a Rust core and a
React/TypeScript renderer.

## Prerequisites

- Windows 10 or later with WebView2
- Node.js `24.18.1` and npm
- Stable Rust with the `x86_64-pc-windows-msvc` toolchain
- Visual Studio Build Tools with the C++ desktop workload
- The official Codex CLI installed globally and signed in

Install the Codex CLI globally when it is not already available:

```powershell
npm install --global @openai/codex
```

After installing Rust for the first time, open a new terminal so that Cargo is on PATH.
Project dependencies remain local to the repository:

```powershell
npm install
```

## Run locally

Start the complete desktop application with the Tauri development server:

```powershell
npm run tauri dev
```

This is the only mode that shows development output: the core's messages appear in the
terminal that started it, and the webview offers right-click → Inspect. The application
stops when that terminal stops. Built executables are windowed applications with no
console attached, so they print nothing anywhere; use this mode to diagnose them.

For renderer-only work, start Vite without the Rust host:

```powershell
npm run dev
```

The renderer-only mode cannot call the Tauri commands that acquire or persist provider
data.

## Build and verify

Run the renderer tests. They cover the shared formatting, date-range, and error helpers.
They assert structure rather than exact formatted text, so the single locale constant in
`src/format.ts` can change when the interface gains a language choice:

```powershell
npm test
```

Run the core tests. They cover status thresholds, error sanitization, quota-window naming,
and the SQLite layer, including that migrations apply and that a history refresh replaces
only the days it parsed:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

Both suites, plus the renderer build, run on a Windows runner for every pull request into
`dev` and every push to `dev` or `main`; see `.github/workflows/ci.yml`.

### What each build command produces

The commands below overlap, and choosing the wrong one is the usual reason a build "works"
but the result cannot be run. Each row states what lands on disk and what it is for.

| Command | Produces | Use it for |
| --- | --- | --- |
| `npm run build` | `dist/` — the compiled renderer only | Type-checking and bundling the interface. Produces no executable |
| `cargo check --manifest-path src-tauri/Cargo.toml` | Nothing on disk | Confirming the core compiles, faster than a build |
| `npm run tauri dev` | A running application, plus a **dev-server-bound** `src-tauri/target/debug/quotastation.exe` | Development and diagnosis. See the caution below |
| `npm run tauri -- build --debug --no-bundle` | `src-tauri/target/debug/quotastation.exe`, standalone | A runnable build with debug assertions and symbols |
| `npm run tauri build -- --no-bundle` | `src-tauri/target/release/quotastation.exe`, standalone | The optimized application, run straight from `target/` |
| `npm run tauri -- build --bundles nsis` | An NSIS installer under `src-tauri/target/release/bundle/nsis/` | Distribution only, as part of an explicitly requested release |

Both `--no-bundle` forms embed `dist/` into the executable, so the file needs nothing beside
it and runs from wherever it sits. The build runs `npm run build` first, so the renderer is
always current. Windows 11 supplies the WebView2 runtime these builds require.

⚠️ `npm run tauri dev` writes its own `quotastation.exe` to the same `target/debug/` path,
and that one loads the interface from `http://localhost:1420` instead of from itself.
Launching it later without the dev server running gives a window with nothing in it. After
running the development server, rebuild with `--debug --no-bundle` before running the debug
executable by hand.

⚠️ Only one instance runs at a time. Starting a second executable hands over to the
instance already running and exits immediately, which looks like the new build crashing.
Close the running copy — including one started from the tray — before launching another.

Every build shares one database and one settings file under
`%APPDATA%\me.stevewang.quotastation`, so a development build sees the installed
application's history, quota, and provider settings.

Use the minimum relevant check for a focused change. Documentation-only changes need only
a review of the edited files.

## Codex executable discovery

QuotaStation discovers `codex` from PATH. To test a different official Codex executable,
set `QUOTASTATION_CODEX_EXECUTABLE` in the launching shell. Do not put machine-specific
paths into committed files; any local configuration file must contain `.local` in its
filename.

## Local data

Normalized data is stored in the application data directory at:

```text
%APPDATA%\me.stevewang.quotastation\quotastation.db
```

The database contains normalized usage, limits, refresh state, and pricing provenance. It
does not store prompts, source code, raw session records, credentials, or full source paths.
Historical range changes query the normalized daily rows in SQLite; they do not trigger a
new parse of the Codex session logs.

Quota samples remain at their roughly five-minute source granularity for 14 days. SQLite
retains hourly rollups through day 60 and daily rollups through day 180, keeping reset
windows separate. Daily usage remains indefinitely. Startup performs this maintenance at
most once every 24 hours; provider refreshes do not run it, and it does not issue `VACUUM`.
Successful refresh diagnostics remain for 30 days and failures for 180 days, while the
newest result for each acquisition path is always preserved.

## Refresh lifecycle and diagnostics

QuotaStation keeps live quota and local history acquisition independent:

- Live quota refreshes at startup, on manual refresh, and on a schedule of its own for each
  provider: every five minutes for Codex, which answers from a local process, and every ten
  minutes for Claude Code, whose window is recovered by parsing its session logs. A change
  to those logs also refreshes Claude's window immediately, alongside its history.
- Claude Code's optional online cross-check is off by default. When it is on it runs with
  the live refresh, never more than once every fifteen minutes, and it always yields to a
  `Retry-After`. Its failures are recorded against their own acquisition path and leave the
  log-derived window on display.
- History refreshes at startup and on manual refresh. A recursive watcher reuses ccusage's
  resolved Codex session locations and debounces `.jsonl` changes for two seconds.
- A full history reconciliation runs every fifteen minutes to recover missed filesystem
  notifications.
- Renderer range changes continue to query SQLite. A successful history refresh emits an
  application event so the active range updates without waiting for the UI polling fallback.

The status bar's Diagnostics panel reads normalized refresh records and in-memory watcher
health. It exposes acquisition status, timestamps, redacted errors, watched-location count,
and embedded source revisions. It never exposes full paths or raw session records.

## Pricing catalog lifecycle

QuotaStation reuses ccusage's pricing build integration instead of maintaining a manual GPT
price list.

- `vendor/ccusage/flake.lock` pins the reviewed LiteLLM pricing revision.
- `vendor/litellm/model_prices_and_context_window.json` is that pinned catalog, reduced to
  the model identifiers ccusage embeds. `.cargo/config.toml` points
  `CCUSAGE_PRICING_JSON_PATH` at it, so builds never download the catalog: the upstream
  fetch allows ten seconds for a 1.6 MB file and a slow connection fails the build.
- On the first clean Rust build, ccusage's build script reads that snapshot and writes a
  generated `litellm-pricing.json` under
  `src-tauri/target/<profile>/build/ccusage-core-*/out/`.
- Cargo embeds the filtered OpenAI/GPT catalog in the binary. The generated file is under
  ignored `target/` output and must not be committed.
- `vendor/ccusage/rust/crates/ccusage-core/src/models-dev-pricing.json` is different: it is
  ccusage's source-controlled override table for development and alias model identifiers.
  The crate embeds it directly with `include_str!`, so it is required and committed.
- Updating the reviewed ccusage revision updates the catalog pin. Refresh the snapshot from
  the revision the new `flake.lock` names, keeping the same model-identifier filter, and
  review the upstream changes, licenses, notices, and the minimal vendored source together.

Cargo reuses its build output during normal incremental builds. Cleaning `target/`, changing
the relevant ccusage build inputs, or moving to a new pinned revision causes the catalog to
be generated again.
