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

## Changelog

- `CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
  Semantic Versioning. Notable user-facing changes land in its `## [Unreleased]` section in
  the same change that makes them, not in a sweep before the release.
- Record the net user-facing result, not a commit log. Omit pure build, CI, formatting,
  test, typo, and version-bump churn unless a person using the application perceives it.
- Use the Keep a Changelog categories in this order — Added, Changed, Deprecated, Removed,
  Fixed, Security — and omit the empty ones.
- Compare links live at the bottom of the file and are maintained by hand: this repository
  has no release script.

## Releasing

Releasing is manual here, and Steve starts it. Never bump a version, tag, create the
`dev` → `main` pull request, or publish a release without being asked.

- The version appears in three files that must always move together: `package.json`,
  `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. `Cargo.lock` records it too, so
  refresh it in the same commit.
- `main` holds the released state and nothing deploys from it — QuotaStation is a desktop
  application, so a release is a tag plus, when asked for, a built bundle. The repository's
  GitHub default branch is `dev`.
- Annotated `vX.Y.Z` tags on `main` are the source of truth for released versions. The tag
  message is the subject line only — `QuotaStation X.Y.Z` — because the notes already live
  in `CHANGELOG.md` and a second copy would drift. Tags carry no AI attribution, exactly as
  commits do.

The full sequence, once Steve asks for it:

1. On a clean `dev`: `git fetch origin` then `git merge --ff-only origin/dev`.
2. Bump the three version fields, refresh `Cargo.lock`, and run the three verification gates.
3. Rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, open a fresh empty `[Unreleased]`
   above it, and update the compare links at the bottom of the file.
4. Commit that as `chore(release): vX.Y.Z` and push `dev`.
5. Open the `dev` → `main` pull request titled `chore(release): vX.Y.Z` — the title becomes
   the squash subject verbatim, so it has to be a Conventional Commit line — with the new
   changelog section as its body.
6. Review the release pull request with the `/code-review` skill and resolve what it finds.
   The global rules require a real review here; a diff scan is not one.
7. Squash-merge it: `gh pr merge --squash --body ""`.
8. `git checkout main && git pull origin main`, then
   `git tag -a vX.Y.Z -m "QuotaStation X.Y.Z"` and `git push origin vX.Y.Z`. Tags do not
   travel with an ordinary push.
9. Publish a GitHub release from the tag, with the changelog section as its notes, only when
   there is a bundle to attach or Steve asks for one.
10. Reset `dev` to `main` — `git checkout dev && git reset --hard main` and
    `git push --force-with-lease origin dev` — so `dev` starts the next version even with it.
