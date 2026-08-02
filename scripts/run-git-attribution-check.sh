#!/bin/sh
set -eu

repo_root=$(git rev-parse --show-toplevel)
checker="$repo_root/scripts/check-git-attribution.mjs"

if command -v bun >/dev/null 2>&1; then
  exec bun "$checker" "$@"
fi

if command -v node >/dev/null 2>&1; then
  exec node "$checker" "$@"
fi

echo "Git attribution check requires Bun or Node.js." >&2
echo "Install the repository prerequisites before committing or pushing." >&2
exit 1
