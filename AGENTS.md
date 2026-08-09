# QuotaStation contributor guidance

This repository follows Steve's global project rules. The rules below are specific to
QuotaStation and take precedence when they differ.

## Project status

- QuotaStation is in foundation and architecture planning.
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
- Confirm license compatibility before copying or adapting code from reference projects.

## Verification

- Until an implementation stack is selected, documentation-only work needs only a focused
  file review.
- Once the application exists, use the minimum local build or start check required by the
  global rules.
