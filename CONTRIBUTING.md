# Contributing to QuotaStation

Thank you for helping improve QuotaStation.

## Before opening an issue

- Search existing issues for the same provider, behavior, or proposal.
- Remove credentials, account identifiers, prompts, source code, and private file paths from
  screenshots and logs.
- For provider changes, explain whether the behavior comes from an official interface, a
  local file format, or something observed in the client.

## Development workflow

QuotaStation uses Tauri 2, Rust, React, TypeScript, Vite, SQLite, and the official Codex CLI.
See [Development](docs/development.md) for setup, build commands, local data locations, and
pricing updates.

1. Start from the repository's `dev` branch.
2. Keep each change focused and reuse the shared Rust data model.
3. Add or update tests appropriate to the changed behavior.
4. Run the documented minimum local verification.
5. Use a one-line Conventional Commit subject.

## Pull requests

Describe what changes for users, which provider or component is affected, how you tested it,
and whether it changes privacy or compatibility. Do not include real usage data or secrets in
a pull request.

By contributing, you agree that your contribution is licensed under the GNU Affero General Public License v3.0 only.
