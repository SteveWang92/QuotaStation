# Development

QuotaStation is a Tauri 2 Windows application with a Rust core and a React/TypeScript
interface.

## Prerequisites

- Windows 10 or later with WebView2
- Node.js `24.18.1` and npm
- Stable Rust with the `x86_64-pc-windows-msvc` toolchain
- Visual Studio Build Tools with the C++ desktop workload
- The official Codex CLI, when testing Codex quota collection

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

Start the complete desktop application with Tauri's development server:

```powershell
npm run tauri dev
```

This is the only mode that shows development output: the core's messages appear in the
terminal that started it, and the webview offers right-click → Inspect. The application
stops when that terminal stops. Built executables are windowed applications with no console
attached, so they print nothing to a terminal; what they and the status-line bridge do is
recorded in `quotastation.log` instead (see Local data), and this mode is for everything
that log does not answer.

For interface-only work, start Vite without the Rust host:

```powershell
npm run dev
```

This mode cannot call the Tauri commands that read or store provider data.

## Build and verify

Run the interface tests. They cover the shared formatting, date-range, and error helpers.
They assert structure rather than exact formatted text, so the single locale constant in
`src/format.ts` can change when the interface gains a language choice:

```powershell
npm test
```

Run the core tests. They cover status thresholds, error sanitization, quota-window naming,
and the SQLite layer, including that migrations apply and that a history refresh replaces
only the days it parsed:

```powershell
npm run test:core
```

Check the interface formatting, import order, and lint rules. `npm run format` writes what
the check reports; the rules this project turns off, and why, are listed in `biome.jsonc`:

```powershell
npm run lint
```

Check the core the same way. Formatting is `rustfmt.toml`; the lint gate treats every clippy
warning as an error, and `cargo fmt --manifest-path src-tauri/Cargo.toml` writes the
formatting:

```powershell
npm run fmt:core
npm run lint:core
```

`.vscode/settings.json` and `.vscode/extensions.json` are committed so an editor formats
this repository the way the gates do: Biome for the interface, resolved from `node_modules`
rather than a global install, and rustfmt through rust-analyzer for the core. They also
switch a globally installed Prettier off for this workspace and stop format-on-save from
touching Markdown, YAML, TOML and SQL, none of which anything here formats — the
documentation is wrapped by hand. Nothing else under `.vscode/` is shared.

Run every interface and core check together before a release pull request with:

```powershell
npm run verify
```

Both test suites and both lint checks, plus the interface build, run on a Windows runner for every
pull request into `dev`, for the `dev` to `main` release pull request, and for every push to
`main`; see `.github/workflows/ci.yml`. A commit pushed straight to `dev` is not covered, so
run the gates above locally before pushing one.

### What each build command produces

The commands below overlap, and choosing the wrong one is the usual reason a build "works"
but the result cannot be run. Each row states what lands on disk and what it is for.

| Command | Produces | Use it for |
| --- | --- | --- |
| `npm run build` | `dist/` — the compiled interface only | Type-checking and bundling the interface. Produces no executable |
| `npm run check:core` | Nothing on disk | Confirming the core compiles, faster than a build |
| `npm run tauri dev` | A running application, plus a **dev-server-bound** `src-tauri/target/debug/quotastation.exe` | Development and diagnosis. See the caution below |
| `npm run build:debug` | `src-tauri/target/debug/quotastation.exe`, standalone | A runnable build with debug assertions and symbols |
| `npm run build:release` | `src-tauri/target/release/quotastation.exe`, standalone | The optimized application, run straight from `target/` |
| `npm run build:installer` | An NSIS installer under `src-tauri/target/release/bundle/nsis/` | Distribution only, as part of an explicitly requested release |

`src-tauri/target/release/quotastation.exe` is the copy the application is actually run
from between releases, so it is the one a finished change is rebuilt and relaunched as. The
debug forms are for diagnosing a specific problem.

Both `--no-bundle` forms embed `dist/` into the executable, so the file needs nothing beside
it and runs from wherever it sits. The build runs `npm run build` first, so the interface is
always current. Windows 11 supplies the WebView2 runtime these builds require.

⚠️ `npm run tauri dev` writes its own `quotastation.exe` to the same `target/debug/` path,
and that one loads the interface from `http://localhost:1420` instead of from itself.
Launching it later without the dev server running gives a window with nothing in it. After
running the development server, rebuild with `--debug --no-bundle` before running the debug
executable by hand.

⚠️ Only one instance runs at a time. Starting a second executable hands over to the
instance already running and exits immediately, which looks like the new build crashing.
Close the running copy — including one started from the tray — before launching another.

A launch with no arguments opens the dashboard. `quotastation.exe --background` comes up in
the tray with no window, which is how Windows starts it at logon; use it to relaunch a build
without taking the foreground, and launch without the argument when the window is what you
are checking.

Every build shares one database and one settings file under
`%APPDATA%\me.stevewang.quotastation`, so a development build sees the installed
application's history, quota, and provider settings.

Use the minimum relevant check for a focused change. Documentation-only changes need only
a review of the edited files.

## Packaging and distribution

`npm run build:installer` produces the per-user NSIS installer. A release does not need it
built by hand: publishing a `vX.Y.Z` release runs `.github/workflows/release.yml`, which
builds the installer from the published tag and attaches it to that release.

The installer and the executable inside it are **unsigned**. A code-signing certificate is a
recurring cost QuotaStation deliberately does not carry, so Microsoft Defender SmartScreen may
warn that a new release is unrecognized. On systems that allow unsigned applications, the
warning can be dismissed through **More info** → **Run anyway** after the download source has
been checked. Managed Windows policies may block that choice. Nothing in the build attempts to
bypass the warning; the installer is built by GitHub Actions from the source tag attached to
the same release.

Installing a newer version keeps the application data and external integrations in place. On
a full uninstall, the NSIS pre-uninstall hook runs the installed executable with
`--uninstall-cleanup`; that mode removes only QuotaStation's own Claude Code status line and
`Stop` hook, then exits before Tauri starts. NSIS owns the Windows startup entry and shortcuts
and removes them itself. The uninstaller leaves `%APPDATA%\me.stevewang.quotastation` in place
unless the user selects **Delete app data**, so reinstalling normally restores the previous
history and settings.

Those three integrations live outside that directory, so keeping the data alone would leave a
reinstall showing them switched off. Before removing anything, `--uninstall-cleanup` writes
`restore-integrations.json` beside the database, recording which of them were actually on; the
next start reads that note once, re-registers what it names, and deletes it. It records the
state at uninstall rather than mirroring the settings because a mirror cannot tell a user who
turned an integration off from an uninstaller that took it away, and it gives up rather than
retries — a status line another program has claimed in the meantime is the user's to sort out.
Selecting **Delete app data** removes the note with everything else, which is correct: an
uninstall that keeps nothing has nothing to restore.

## Application icon

`src-tauri/icons/` holds the square `app-icon.png` master artwork, the crisp
`app-icon-small.svg` used from 16 through 96 pixels, and `icon.ico`, the only icon a Windows
build bundles. Replace the applicable source artwork and run

```powershell
npm run icons
```

which generates both sources into scratch directories, combines their Windows sizes into the
`.ico`, and discards the Android, iOS, macOS and Store variants `tauri icon` also produces.
Running `npx tauri icon` directly writes all forty of them into the repository instead. The
Rust build script watches the generated `.ico`, so the next build also refreshes the icon
embedded in the executable rather than only the icon Tauri loads at runtime.

The generated ICO carries exact Windows 11 target sizes at every scale factor: 16, 20, 24,
30, 32, 36, 40, 48, 60, 64, 72, 80, 96 and 256 pixels. `resize-icon.ps1` runs only inside the
Windows-only icon task and keeps those derived PNGs in the scratch directory.

## Codex executable discovery

QuotaStation discovers `codex` from PATH. To test a different official Codex executable,
set `QUOTASTATION_CODEX_EXECUTABLE` in the launching shell. Do not put machine-specific
paths into committed files; any local configuration file must contain `.local` in its
filename.

## Demonstration data

A normal QuotaStation screenshot contains private quota and usage data. `--demo` opens the
application with a fictional data set that is safe to publish:

```powershell
cargo run --example seed_demo --manifest-path src-tauri/Cargo.toml
src-tauri\target\release\quotastation.exe --demo
```

The seeder writes `quotastation-demo.db` and `settings-demo.json` beside the real files in the
application data directory. It uses the application's normal storage code, so the generated
database always matches the current schema. Running it again replaces the demo data. A fixed
seed keeps later screenshots consistent.

A demo instance does not read any provider, watch session files, or refresh in the background.
It runs beside the normal application and uses separate database and settings files, so it
cannot replace or expose real history.

The same fictional data appears in the dashboard, tray, quick panel, and taskbar widget.

## Local data

QuotaStation stores its processed data at:

```text
%APPDATA%\me.stevewang.quotastation\quotastation.db
```

The database contains usage totals, quota readings, refresh results, and the pricing source. It
does not store prompts, source code, raw session records, credentials, or full source paths.

The same directory holds `quotastation.log`. Built executables are windowed applications with
no console, and the status-line bridge is a process that lives for milliseconds inside Claude
Code, so the log is the only place either of them can report what happened: which source
answered a refresh, how many windows it carried, and why a read failed. It records no session
content and no credential, rolls over at 512 KB into `quotastation.log.1`, and the
Diagnostics tab's **Show activity log** button reveals it.

Changing the selected history range queries the stored SQLite rows — daily, or hourly for a range
short enough to be drawn that way; they do not trigger a new parse of the Codex session logs.

What is retained and for how long, how daily buckets follow the system time zone, and why
each rule is what it is are all in
[Architecture — Local database](architecture.md#local-database). Retention runs at startup
and every 24 hours, never during a refresh, and never issues `VACUUM`.

See [Multi-machine usage](multi-machine.md) for sharing totals between computers.

## Refresh and diagnostics

The [architecture guide](architecture.md#provider-data) explains where each provider's data
comes from. When running the application locally:

- Live quota refreshes at startup, on manual refresh, every five minutes for Codex and every
  ten minutes for Claude Code, and immediately whenever Claude Code's session logs change.
- History refreshes at startup and on manual refresh. A recursive watcher waits two seconds
  after `.jsonl` changes settle, and a full scan every fifteen minutes recovers missed Windows
  file notifications. The interface updates after the complete refresh rather than showing
  partially updated totals.
- The Claude Code status-line bridge is installed from the settings page, which registers
  `quotastation.exe --claude-statusline` as that command. Readings then arrive from terminal
  sessions only; the settings card says so when every running session is desktop-hosted.
- The Diagnostics section shows recent refresh results and file-watcher health. Its Settings
  button is marked whenever a provider source, watcher, or command fails.
- Diagnostics show the seven-character source commit beside the application version. The
  Rust build watches the active Git ref, so rebuilding after a commit refreshes that value.

## Updating pricing data

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
