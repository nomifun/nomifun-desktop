# Repository Rules

## GitHub Actions Are Absolutely Forbidden

This is a non-negotiable, repository-wide rule. It applies to every branch,
worktree, contributor, coding agent, and automated tool.

- Never create, restore, generate, stage, commit, merge, or rename a GitHub
  Actions workflow into `.github/workflows/`.
- No `.yml` or `.yaml` workflow file is allowed in that directory. Disabled,
  manual-only, scheduled, reusable, release-only, and temporary workflows are
  all prohibited without exception.
- Never enable GitHub Actions in the repository settings or through the GitHub
  API or CLI.
- Do not bypass this rule by placing equivalent GitHub Actions configuration in
  another path, generating it during release work, or restoring it from Git
  history.
- Build, test, packaging, and release automation must use local scripts and
  documented manual commands instead of repository-hosted workflows.
- Historical documentation that mentions CI or GitHub Actions is descriptive
  only and does not override this rule.

Before completing any change, verify that no workflow YAML exists under
`.github/workflows/`. If a task conflicts with this rule, stop and report the
conflict. The rule must be explicitly changed by the repository owner before
such work can proceed.

## Git Attribution Must Identify a Human

This is a repository-local rule for `nomifun-tauri`. Do not change any
developer's global Git identity or global Git configuration to enforce it, and
do not apply it to unrelated repositories.

Every commit must attribute the work to the responsible human developer. AI
tools may assist with a change, but they must never appear as the author,
committer, co-author, or other credited contributor.

- Never use an AI model, AI product, vendor, bot, or agent identity in the Git
  author or committer name/email. Prohibited identities include, but are not
  limited to, Claude, Codex, GPT, ChatGPT, Gemini, Copilot, OpenAI, and
  Anthropic.
- Never add AI-credit trailers or equivalent attribution to a commit message,
  including `Co-authored-by`, `Generated-by`, `Assisted-by`, or similar lines.
  Technical references to an AI model or product remain allowed when they are
  genuinely part of the change being described.
- After cloning this repository, run `bun run setup:git-hooks` to enable the
  repository-local attribution checks. Never bypass those checks with
  `--no-verify`.
- Preserve the known human author and committer when amending or rewriting
  history. If the responsible human cannot be determined, use
  `NomiFun Contributor <nomifun@users.noreply.github.com>` as both author and committer.
- Before committing, amending, rebasing, cherry-picking, or pushing rewritten
  history, inspect the affected commits and verify that their author,
  committer, and attribution trailers comply with this rule.
