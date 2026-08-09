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

For renderer-only work, start Vite without the Rust host:

```powershell
npm run dev
```

The renderer-only mode cannot call the Tauri commands that acquire or persist provider
data.

## Build and verify

Build the renderer:

```powershell
npm run build
```

Check the Rust application:

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

Build a local debug executable without producing an installer:

```powershell
npm run tauri -- build --debug --no-bundle
```

The executable is written to `src-tauri/target/debug/quotastation.exe`. Producing the NSIS
installer is a release task and should only be done as part of an explicitly requested
release workflow:

```powershell
npm run tauri -- build --bundles nsis
```

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

## Refresh lifecycle and diagnostics

QuotaStation keeps live quota and local history acquisition independent:

- Live quota refreshes at startup, on manual refresh, and every five minutes.
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
- On the first clean Rust build, ccusage downloads that pinned catalog and writes a generated
  `litellm-pricing.json` under
  `src-tauri/target/<profile>/build/ccusage-core-*/out/`.
- Cargo embeds the filtered OpenAI/GPT catalog in the binary. The generated file is under
  ignored `target/` output and must not be committed.
- `vendor/ccusage/rust/crates/ccusage-core/src/models-dev-pricing.json` is different: it is
  ccusage's source-controlled override table for development and alias model identifiers.
  The crate embeds it directly with `include_str!`, so it is required and committed.
- Updating the reviewed ccusage revision updates the catalog pin. Review the upstream
  changes, licenses, notices, and the minimal vendored source together.

Cargo reuses its build output during normal incremental builds. Cleaning `target/`, changing
the relevant ccusage build inputs, or moving to a new pinned revision causes the catalog to
be generated again.
