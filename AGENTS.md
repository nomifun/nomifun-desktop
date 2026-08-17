# Repository Guidelines

## Efficient validation

- Choose the smallest checks that directly cover the files and behavior being
  changed. Documentation-only edits do not require a full build or test run.
- Use targeted UI, Rust, packaging, or platform checks for changes in those
  areas. Run broad commands such as `bun run check` or the full test suite for
  cross-cutting changes, releases, or when a narrower check cannot establish
  confidence.
- If a relevant command is unavailable or platform-specific, report what was
  not run and why. It is not a repository-wide commit blocker for unrelated
  changes.
- Repository scripts and GitHub Actions may be used when they are useful and
  maintainable; no workflow-presence audit is required for ordinary changes.

## Git workflow

- Use the contributor's configured Git identity. The repository does not
  install or require custom attribution hooks.
- Preserve unrelated work and inspect staged files before committing.
- Do not force-push, rewrite shared history, or expose credentials without
  explicit authorization.
