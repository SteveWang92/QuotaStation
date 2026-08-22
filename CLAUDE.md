# QuotaStation contributor guidance

This repository follows Steve's global project rules. The rules below are specific to
QuotaStation and take precedence when they differ.

## Project status

- QuotaStation is released and under active development. Codex and Claude are both covered;
  the repository is private. Which version is current is a question for the tags and
  `CHANGELOG.md`, not for this file.
- Read `docs/PROJECT_PLAN.local.md` when it exists before changing implementation scope.
- Keep public documentation free of machine-specific paths, account details, credentials,
  and private usage data.

## Which document owns what

Every fact is explained in exactly one of these; the others link to it rather than repeating
it, and a paragraph found in two of them is a bug in the documentation.

| Document | Owns |
| --- | --- |
| `README.md` | What QuotaStation is and what it does today, for someone who has never seen it. No version numbers, no design rationale. |
| `docs/architecture.md` | Why the boundaries are where they are: the stack, the provider/renderer split, data retention, privacy rules, what each source may and may not do. |
| `docs/development.md` | How to run, build, verify, and where local data lives. Commands and paths. |
| `docs/PROJECT_PLAN.local.md` | What is done, what is next, and the decisions and dead ends behind both. The only home for progress. |
| `CHANGELOG.md` | What changed for a user, per version. |
| `CLAUDE.md` | How to work in this repository. |

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
- Use Tauri 2 with a Rust core and a React/TypeScript renderer unless Steve explicitly
  approves an architecture change.
- For AI client logs and provider behavior, inspect established open-source implementations
  first and directly reuse compatible code when practical.
- Pin every reused implementation to an audited revision and record its license,
  attribution, security behavior, and local changes in `THIRD_PARTY_NOTICES.md`.
- Do not add vendor hash manifests, whole-tree integrity hashes, or CI hash verification
  unless Steve explicitly approves that maintenance cost first.
- Do not reimplement the Codex log parser unless direct use or minimal vendoring of the
  reviewed `ccusage` Rust adapter is blocked by a concrete incompatibility.
- Keep file, process, provider protocol, credential, and database access in the Rust core;
  the renderer receives only normalized data through narrow commands and events.

## Verification

- Documentation-only work needs only a focused file review.
- Use the minimum local check required by the global rules. The gates are `npm run lint`,
  `npm test`, `npm run build`, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`,
  `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`,
  and `cargo test --locked --manifest-path src-tauri/Cargo.toml`.
- **CI runs those gates on the release path only** — the `dev` -> `main` pull request and
  `main` itself — because a Windows runner bills at twice its wall clock and a cold Rust
  build dominates it. Nothing checks a `dev` commit but the local run above, so running it
  is not optional here.
- `npm run format` writes the renderer's formatting and import order; `cargo fmt` does the
  same for the core. Run them rather than hand-correcting what the gate reports. A rule the
  code deliberately breaks is turned off in `biome.jsonc` with the reason beside it — never
  with an inline suppression comment.
- **The verification artifact is the unbundled release build, never the debug one.** Steve
  runs `src-tauri/target/release/quotastation.exe` — a debug build is a different binary with
  different performance, and handing him one is handing him something he does not run. The
  `--debug` and `npm run tauri dev` forms in `docs/development.md` exist for diagnosing a
  specific problem, not for finishing a change.
- After the gates pass, close the running instance, rebuild it with `npm run build` then
  `cargo build --release --manifest-path src-tauri/Cargo.toml`, and start it again **in the
  background** — always with `--background`, which comes up in the tray and opens no window:

  ```powershell
  (New-Object -ComObject Shell.Application).ShellExecute(
    "<repo>\src-tauri\target\release\quotastation.exe", "--background")
  ```

  The argument is the point. A launch with no arguments opens the dashboard and takes over
  Steve's screen for a restart he did not ask for; the logon entry carries `--background` for
  that reason, and a verification start is no different. The COM call is what carries an
  argument while leaving the process detached from the agent's terminal, which
  `explorer.exe <path>` cannot do and `Start-Process` does not do. Keep the executable path
  stable: Claude Code's registered status-line command points at it.
- Only one instance runs at a time — a second one hands over to the first and exits, which
  looks like a crash. Close the running copy, including one started from the tray, first.

## Changelog

- `CHANGELOG.md` is the release history and follows the changelog rules in Steve's global
  `CLAUDE.md`, which is where they are explained: user-facing results only, one entry to one
  line, Keep a Changelog categories in order.
- The one difference here: this repository has no release script, so the compare links at the
  bottom of the file are maintained by hand.

## Releasing

Releasing is manual here, and Steve starts it. Never bump a version, tag, create the
`dev` → `main` pull request, or publish a release without being asked.

- The version appears in three files that must always move together: `package.json`,
  `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. `Cargo.lock` and
  `package-lock.json` record it too, so refresh them in the same commit.
- **The release commit is the first commit of the release.** The version bump and the
  changelog dating are committed and pushed *before* the pull request is opened, so the
  review and CI both run against the exact tree that will be tagged; review fixes then land
  as ordinary commits on top of it. The squash merge is what makes the order irrelevant —
  the branch becomes one commit on `main` and the tag goes on `main` afterwards. QuotaStation
  has no release script, so this sequence is run by hand, matching what `release.mjs` /
  `release.py` do in the scripted repositories.
- `main` holds the released state and nothing deploys from it — QuotaStation is a desktop
  application, so a release is a tag and its changelog notes. The repository's GitHub default
  branch is `dev`.
- **No installer is attached to a release while the repository is private.** A bundle is
  built for Steve's own verification, not for distribution, and uploading one before the
  project is public serves nobody. Do not build or attach one, and do not ask each time;
  when QuotaStation goes public, Steve will say so and that is when the artifact question
  reopens.
- Annotated `vX.Y.Z` tags on `main` are the source of truth for released versions. The tag
  message is the subject line only — `QuotaStation X.Y.Z` — because the notes already live
  in `CHANGELOG.md` and a second copy would drift. Tags carry no AI attribution, exactly as
  commits do.

The full sequence, once Steve asks for it:

1. On a clean `dev`: `git fetch origin` then `git merge --ff-only origin/dev`. Run the
   verification gates.
2. Bump the three version fields, refresh `Cargo.lock` and `package-lock.json`, rename
   `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, open a fresh empty `[Unreleased]` above
   it, update the compare links at the bottom of the file, re-run the gates, and commit it
   as `chore(release): vX.Y.Z`. Push `dev`.
3. Open the `dev` → `main` pull request titled `chore(release): vX.Y.Z` — the title becomes
   the squash subject verbatim, so it has to be a Conventional Commit line — with that
   version's finished changelog section as its body.
4. Review the release pull request with the `/code-review` skill and resolve what it finds.
   The global rules require a real review here; a diff scan is not one. Fixes are ordinary
   commits pushed to `dev` on top of the release commit; the pull request updates itself. If
   the review concludes the release should be a different level, change the version in every
   place at once — the three version fields, both lockfiles, the changelog heading, the
   compare links and the pull request title — in the same commit as the fix that caused it.
5. Wait for Steve to confirm the pull request is ready to merge. Nothing below this line
   happens before that confirmation.
6. Verify the pull request can actually merge before merging it:
   `gh pr view <N> --json headRefOid,mergeable,mergeStateStatus,statusCheckRollup`. Merge
   only when `mergeStateStatus` is `CLEAN` and `headRefOid` matches local `dev`; anything
   else means checks are still running, a check failed, or `dev` has unpushed work. Wait and
   re-check rather than merging around it.
7. Squash-merge it: `gh pr merge --squash --body ""`.
8. `git checkout main && git pull origin main`, then
   `git tag -a vX.Y.Z -m "QuotaStation X.Y.Z"` and `git push origin vX.Y.Z`. Tags do not
   travel with an ordinary push.
9. Publish a GitHub release from the tag — `gh release create vX.Y.Z --title vX.Y.Z
   --notes-file <section>` — with that version's changelog section as its notes and no
   attached artifact. Every tag gets a release; a tag on its own is not the published record.
10. Reset `dev` to `main` — `git checkout dev && git reset --hard main` and
    `git push --force-with-lease origin dev` — so `dev` starts the next version even with it.

`v0.1.0` was the base case and skipped steps 2 through 6. There was no `main` to diff
against, so there was no pull request and nothing for a release review to gate; the version
and the changelog were already final, so a `chore(release)` commit would have carried no
change at all. `main` was branched from `dev` and the tag placed on the commit they shared,
which also left step 10 with nothing to do. Every release after it follows the full sequence
above. The GitHub default branch stays `dev` so the repository opens on current work.
