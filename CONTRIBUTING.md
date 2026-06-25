# Contributing to NomiFun

NomiFun is a Rust + Tauri + React monorepo. This file is the open-source entry
point for contributors; the detailed engineering docs live under `docs/`.

## Start Here

- [Project structure](docs/contributing/project-structure.md)
- [Development workflow](docs/contributing/development.md)
- [Building and packaging](docs/contributing/building-and-packaging.md)
- [Architecture overview](docs/architecture/overview.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)

## Local Setup

```bash
bun install
bun run dev
```

Use `bun run dev:web` for the browser-only development loop and
`bun run serve:web` to run the headless server after building the SPA.

## Checks

Run the narrowest check that covers your change:

```bash
bun run help --check
bun run typecheck
bun run check
cargo test
```

For UI-only work, prefer package-relative Bun commands from `ui/` or
`bun run --filter=./ui ...` from the repo root. For Rust-only work, use the
specific crate or test target where possible before running the full suite.

## Documentation Changes

- Keep current docs under `docs/getting-started`, `docs/guides`,
  `docs/architecture`, `docs/reference`, and `docs/contributing`.
- Design and audit history is not kept in the repo; consult git history for past
  decisions rather than re-adding dated design docs.
- Do not document redirected legacy routes as primary navigation.
- Keep English and Simplified Chinese siblings in sync when both exist.

## Pull Request Expectations

- Keep changes scoped and explain user-visible behavior.
- Include screenshots for visible UI changes when practical.
- Update docs and screenshot manifest rows when routes, setup steps, or
  feature names change.
- Do not commit local data directories, generated build output, credentials, or
  machine-specific configuration.
- Do not add third-party assets or vendored skills unless their redistribution
  license is compatible with this repository.

## Releases

Maintainer release steps live in [RELEASING.md](RELEASING.md). User-facing
changes should be summarized in [CHANGELOG.md](CHANGELOG.md).

## License

By contributing, you agree that your contribution is licensed under the
Apache-2.0 license used by this repository.
