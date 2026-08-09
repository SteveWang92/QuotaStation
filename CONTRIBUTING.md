# Contributing to QuotaStation

Thank you for helping improve QuotaStation.

## Before opening an issue

- Search existing issues for the same provider, behavior, or proposal.
- Remove credentials, account identifiers, prompts, source code, and private file paths from
  screenshots and logs.
- For provider changes, identify whether the behavior comes from an official interface, a
  local client format, or an observed implementation detail.

## Development workflow

QuotaStation uses Tauri 2, Rust, React, TypeScript, Vite, SQLite, and the official Codex CLI.
See [Development](docs/development.md) for environment setup, run and build commands, local
data locations, and the pricing catalog lifecycle.

1. Start from the repository's `dev` branch.
2. Keep each change focused and reuse the shared normalization core.
3. Add or update tests appropriate to the changed behavior.
4. Run the documented minimum local verification.
5. Use a one-line Conventional Commit subject.

## Pull requests

Describe the user-visible outcome, the provider or component affected, verification
performed, and any privacy or compatibility implications. Do not include real usage data or
secrets in a pull request.

By contributing, you agree that your contribution is licensed under the GNU Affero General Public License v3.0 only.
