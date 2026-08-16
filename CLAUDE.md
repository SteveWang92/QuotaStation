# QuotaStation contributor guidance

This repository follows Steve's global project rules. The rules below are specific to
QuotaStation and take precedence when they differ.

## Project status

- QuotaStation is implemented and preparing its first release. Codex and Claude are both
  covered; the repository is private.
- Read `docs/PROJECT_PLAN.local.md` when it exists before changing implementation scope.
- Keep public documentation free of machine-specific paths, account details, credentials,
  and private usage data.

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
- Use the minimum local check required by the global rules. The three gates CI enforces are
  `npm test`, `npm run build`, and `cargo test --locked --manifest-path src-tauri/Cargo.toml`.
  There is no lint or format gate.
- **The verification artifact is the unbundled release build, never the debug one.** Steve
  runs `src-tauri/target/release/quotastation.exe` — a debug build is a different binary with
  different performance, and handing him one is handing him something he does not run. The
  `--debug` and `npm run tauri dev` forms in `docs/development.md` exist for diagnosing a
  specific problem, not for finishing a change.
- After the gates pass, close the running instance, rebuild it with `npm run build` then
  `cargo build --release --manifest-path src-tauri/Cargo.toml`, and start it again with
  `explorer.exe src-tauri\target\release\quotastation.exe` so it comes up owned by the shell
  exactly as a double-click would rather than tied to the agent's terminal. Keep that
  executable path stable: Claude Code's registered status-line command points at it.
- Only one instance runs at a time — a second one hands over to the first and exits, which
  looks like a crash. Close the running copy, including one started from the tray, first.

## Changelog

- `CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
  Semantic Versioning. Notable user-facing changes land in its `## [Unreleased]` section in
  the same change that makes them, not in a sweep before the release.
- Record the net user-facing result, not a commit log. Omit pure build, CI, formatting,
  test, typo, and version-bump churn unless a person using the application perceives it.
- **One entry is one line — a single sentence naming the result, and nothing else.** No
  second sentence, no wrapped continuation line, no reason, no mechanism, no before-and-after,
  no list of what stayed the same. If an entry does not fit on one line it is carrying
  explanation that belongs in the code comment or the commit, not here. The reader wants to
  know what changed for them, and every extra clause is one more line they read to find it.
- Use the Keep a Changelog categories in this order — Added, Changed, Deprecated, Removed,
  Fixed, Security — and omit the empty ones.
- Compare links live at the bottom of the file and are maintained by hand: this repository
  has no release script.

## Releasing

Releasing is manual here, and Steve starts it. Never bump a version, tag, create the
`dev` → `main` pull request, or publish a release without being asked.

- The version appears in three files that must always move together: `package.json`,
  `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. `Cargo.lock` and
  `package-lock.json` record it too, so refresh them in the same commit.
- **The release commit is the last commit before the merge.** The pull request is opened,
  reviewed and fixed on the unbumped branch; the version bump and the changelog dating are
  committed only after Steve confirms the pull request is ready to merge. A release commit
  pushed before the review, or left sitting under later fixes, means the tagged commit is
  not the state that was reviewed — drop it and force-push with lease if it happens.
- `main` holds the released state and nothing deploys from it — QuotaStation is a desktop
  application, so a release is a tag plus, when asked for, a built bundle. The repository's
  GitHub default branch is `dev`.
- Annotated `vX.Y.Z` tags on `main` are the source of truth for released versions. The tag
  message is the subject line only — `QuotaStation X.Y.Z` — because the notes already live
  in `CHANGELOG.md` and a second copy would drift. Tags carry no AI attribution, exactly as
  commits do.

The full sequence, once Steve asks for it:

1. On a clean `dev`: `git fetch origin` then `git merge --ff-only origin/dev`. Run the three
   verification gates.
2. Open the `dev` → `main` pull request titled `chore(release): vX.Y.Z` — the title becomes
   the squash subject verbatim, so it has to be a Conventional Commit line — with the
   `[Unreleased]` entries as its body. **No version bump yet:** `dev` still carries the
   previous version at this point.
3. Review the release pull request with the `/code-review` skill and resolve what it finds.
   The global rules require a real review here; a diff scan is not one. Fixes are ordinary
   commits pushed to `dev`; the pull request updates itself.
4. Wait for Steve to confirm the pull request is ready to merge. Nothing below this line
   happens before that confirmation.
5. Now bump the three version fields, refresh `Cargo.lock` and `package-lock.json`, rename
   `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, open a fresh empty `[Unreleased]` above
   it, update the compare links at the bottom of the file, re-run the three gates, and
   commit it as `chore(release): vX.Y.Z`. Push `dev`. This is the last commit on the branch.
6. Update the pull request body to the finished changelog section if the entries changed
   during the review.
7. Squash-merge it: `gh pr merge --squash --body ""`.
8. `git checkout main && git pull origin main`, then
   `git tag -a vX.Y.Z -m "QuotaStation X.Y.Z"` and `git push origin vX.Y.Z`. Tags do not
   travel with an ordinary push.
9. Publish a GitHub release from the tag — `gh release create vX.Y.Z --title vX.Y.Z
   --notes-file <section>` — with that version's changelog section as its notes, and attach
   the built bundle when there is one. Every tag gets a release; a tag on its own is not the
   published record.
10. Reset `dev` to `main` — `git checkout dev && git reset --hard main` and
    `git push --force-with-lease origin dev` — so `dev` starts the next version even with it.

`v0.1.0` was the base case and skipped steps 2 through 6. There was no `main` to diff
against, so there was no pull request and nothing for a release review to gate; the version
and the changelog were already final, so a `chore(release)` commit would have carried no
change at all. `main` was branched from `dev` and the tag placed on the commit they shared,
which also left step 10 with nothing to do. Every release after it follows the full sequence
above. The GitHub default branch stays `dev` so the repository opens on current work.
