# QuotaStation contributor guidance

This is the authoritative guidance for working in QuotaStation, for a coding agent and a
human contributor alike. The maintainer's own cross-project rules live outside this
repository; where they and the rules below differ, the rules below win.

## Project status

- QuotaStation is released and under active development. Codex and Claude are both covered.
  Which version is current is a question for the tags and `CHANGELOG.md`, not for this file.
- Read `docs/PROJECT_PLAN.local.md` when it exists before changing implementation scope.
- Keep public documentation free of machine-specific paths, account details, credentials,
  and private usage data.

## Which document owns what

Every fact is explained in exactly one of these; the others link to it rather than repeating
it, and a paragraph found in two of them is a bug in the documentation.

Every document and code comment describes the current state: no dates, progress, version
history, or account of how the code used to behave. Those belong in issues, commits, and
`CHANGELOG.md`.

| Document | Owns |
| --- | --- |
| GitHub issues | Planned work and its progress. The only home for progress. |
| `README.md` | What QuotaStation is and what it does today, for someone who has never seen it. No version numbers, no design rationale. |
| `docs/architecture.md` | Why the boundaries are where they are: the stack, the provider/renderer split, data retention, privacy rules, what each source may and may not do. |
| `docs/development.md` | How to run, build, verify, and where local data lives. Commands and paths. |
| `docs/PROJECT_PLAN.local.md` | Product direction, guardrails, and the accepted decisions and dead ends behind the current design. Never progress. |
| `CHANGELOG.md` | What changed for a user, per version. |
| `CLAUDE.md` | How to work in this repository. |
| `CLAUDE.local.md` | Facts true of one machine only: where its working copy and its running instance live. Never rules. |

## Local-only files

- Mark every local-only file with `.local` in its filename.
- Rely on the repository-wide `*.local` and `*.local.*` ignore rules.
- Do not add a one-off `.gitignore` entry for an individual local file.

## Product constraints

- Keep provider integrations read-only unless a future feature is explicitly approved.
- Never expose provider credentials, prompts, source code, file paths, or raw session data
  outside the local machine.
- Reuse one normalized provider and usage model across tray, widget, and dashboard surfaces.
- Deliver Codex first; Claude, Gemini, and other providers must not block the first release.
- Use Tauri 2 with a Rust core and a React/TypeScript renderer unless the maintainer
  explicitly approves an architecture change.
- For AI client logs and provider behavior, inspect established open-source implementations
  first and directly reuse compatible code when practical.
- Pin every reused implementation to an audited revision and record its license,
  attribution, security behavior, and local changes in `THIRD_PARTY_NOTICES.md`.
- Do not add vendor hash manifests, whole-tree integrity hashes, or CI hash verification
  unless the maintainer explicitly approves that maintenance cost first.
- Do not reimplement the Codex log parser unless direct use or minimal vendoring of the
  reviewed `ccusage` Rust adapter is blocked by a concrete incompatibility.
- Keep file, process, provider protocol, credential, and database access in the Rust core;
  the renderer receives only normalized data through narrow commands and events.

## Verification

- Documentation-only work needs only a focused file review.
- Run the minimum local check that proves the change works, and no more. The gates are `npm run lint`,
  `npm test`, `npm run build`, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`,
  `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`,
  and `cargo test --locked --manifest-path src-tauri/Cargo.toml`.
- **CI runs those gates on every pull request** — into `dev` and into `main` — and on `main`
  itself. A push straight to `dev` is deliberately not covered, because a Windows runner
  bills at twice its wall clock and a cold Rust build dominates it. Nothing checks such a
  commit but the local run above, so running it is not optional here.
- `npm run format` writes the renderer's formatting and import order; `cargo fmt` does the
  same for the core. Run them rather than hand-correcting what the gate reports. A rule the
  code deliberately breaks is turned off in `biome.jsonc` with the reason beside it — never
  with an inline suppression comment.
- **The verification artifact is the unbundled release build, never the debug one.** The
  maintainer runs `src-tauri/target/release/quotastation.exe` between releases — a debug
  build is a different binary with different performance, so handing one over is handing over
  something nobody runs. The `--debug` and `npm run tauri dev` forms in
  `docs/development.md` exist for diagnosing a specific problem, not for finishing a change.
- After the gates pass, close the running instance, rebuild it with `npm run build` then
  `cargo build --release --manifest-path src-tauri/Cargo.toml`, and start it again **in the
  background** — always with `--background`, which comes up in the tray and opens no window:

  ```powershell
  (New-Object -ComObject Shell.Application).ShellExecute(
    "<repo>\src-tauri\target\release\quotastation.exe", "--background")
  ```

  The argument is the point. A launch with no arguments opens the dashboard and takes over
  the screen for a restart nobody asked for; the logon entry carries `--background` for that
  reason, and a verification start is no different. The COM call is what carries an argument
  while leaving the process detached from the agent's terminal, which `explorer.exe <path>`
  cannot do and `Start-Process` does not do. Keep the executable path stable — what points
  at it on a given machine is in that machine's `CLAUDE.local.md`.
- Only one instance runs at a time — a second one hands over to the first and exits, which
  looks like a crash. Close the running copy, including one started from the tray, first.

## Changelog

- `CHANGELOG.md` is the release history: user-facing results only, one entry to one line,
  Keep a Changelog categories in order, and nothing about commits or internal churn.
- `release:prep` finalizes the `[Unreleased]` section into a versioned entry and maintains
  the compare links at the bottom of the file.

## Releasing

Releasing is manual here, and the maintainer starts it. Never bump a version, tag, create
the `dev` → `main` pull request, or publish a release without being asked.

`scripts/release.mjs` drives it through the active release skill — `npm run release:prep`,
`npm run release:reversion -- X.Y.Z`, `npm run release:ship`, each accepting `--dry-run`.
The script is the authoritative implementation for this repository; the shared
`prep` / `reversion` / `ship` workflow lives only in the maintainer’s global guidance. Run
the verification gates yourself before `prep` — the script runs no build and no tests.

What is particular to this repository:

- **Five files carry the version and move together**: `package.json`, `package-lock.json`,
  `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and `src-tauri/Cargo.lock`. They are
  the script’s `VERSION_FIELDS`; `prep` writes all five and `ship`’s pre-flight refuses a
  field left behind. Change a prepped version only with `reversion`.
- **`main` holds the released state and nothing deploys from it** — QuotaStation is a
  desktop application, so a release is a tag, its changelog notes, and the installer CI
  attaches to it. The repository’s GitHub default branch is `dev`.
- **The installer is attached by CI, not by hand.** Publishing the release runs
  `.github/workflows/release.yml`, which builds the per-user NSIS installer from the
  published tag and uploads it to that release. `ship` prints the command that checks that
  run; confirm it finished and the installer is on the release before calling the release
  done. Do not build or upload a bundle manually; if the workflow fails, fix it or re-run it
  from the tag rather than attaching a local build nobody can trace to a tree. The installer
  is unsigned on purpose — `docs/development.md` owns that decision and what it means for
  the people who download it.
- Annotated `vX.Y.Z` tags on `main` are the source of truth for released versions. The tag
  message is the subject line only — `QuotaStation X.Y.Z` — because the notes already live
  in `CHANGELOG.md` and a second copy would drift. Tags carry no AI attribution, exactly as
  commits do.
- Review the release pull request with the `/code-review` skill and resolve what it finds;
  the global rules require a real review here, and a diff scan is not one. Review fixes are
  ordinary commits on `dev` on top of the release commit — re-run `prep` to refresh the PR.
- Nothing past the review happens before the maintainer confirms the pull request is ready
  to merge.
