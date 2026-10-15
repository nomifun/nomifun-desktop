# Contributing to NomiFun

[简体中文](CONTRIBUTING.zh-CN.md)

Read [AGENTS.md](AGENTS.md) for repository boundaries and verification rules.
Use the contributor's configured Git identity and preserve unrelated work.

## Start here

| Task | Maintained guide |
| --- | --- |
| Set up a development environment | [Development](docs/contributing/development.md) |
| Find the owning module | [Project structure](docs/contributing/project-structure.md) |
| Change renderer behavior | [Frontend architecture](docs/architecture/frontend.md) and [styling](docs/contributing/styling.md) |
| Change schema, IDs, reset or restore | [Data and identifier standards](docs/contributing/data-and-identifier-standards.md) |
| Change Agent storage or content | [Canonical Agent Session](docs/architecture/agent-session.md) |
| Build or package | [Building and packaging](docs/contributing/building-and-packaging.md) |
| Prepare a release | [Release runbook](RELEASING.md) |

## Contribution expectations

- Keep each pull request focused on one problem. Discuss product direction,
  architecture, database changes, security boundaries and new dependencies early.
- Follow existing ownership and patterns. Backend feature logic belongs in its
  owning crate; renderer host operations go through the platform/adapter layer.
- Support desktop-class windows and WebUI browsers at 880x600 or larger. Use
  container-aware layout for narrow panes; do not add mobile renderer layouts.
- Update both English and Simplified Chinese translations and sibling docs
  when behavior changes.
- Keep current documentation in the maintained guide/reference/architecture
  directories. Remove superseded plans and duplicate explanations; Git history
  preserves them. Do not create a second archive of development instructions.
- Do not commit credentials, private data, generated build output or assets
  without redistribution rights. Contributions use the repository's Apache-2.0
  license.
- Follow the [code of conduct](CODE_OF_CONDUCT.md). Report vulnerabilities
  privately through [SECURITY.md](SECURITY.md).
- AI-assisted contributions have the same review and correctness obligations as
  other contributions.

## Validate and submit

Run the smallest checks that cover the change; the command map is in
[Development](docs/contributing/development.md#verification). Documentation
changes need `bun run check:docs` and `git diff --check`; they do not need a
full build. Record checks that could not run and why.

Use a specific commit title; Conventional Commits are preferred, for example
`fix(conversation): preserve selected model after retry`. Before review:

- Explain the problem, resulting behavior and material risks.
- List the checks actually run; include screenshots for visible UI changes.
- Update docs, both locales, and `CHANGELOG.md` when users or operators need
  release notes.
- Inspect the diff and staged files for unrelated changes and private data.
- Link issues accurately: `Fixes` for complete resolutions, `Refs` for partial work.