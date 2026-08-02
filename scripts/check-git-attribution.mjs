import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const MODEL_IDENTITY_PATTERN =
  /(^|[^a-z0-9])(claude|codex|chatgpt|gpt(?:[-_. ]?[0-9][a-z0-9.-]*)?|gemini|copilot|openai|anthropic)(?=$|[^a-z0-9])/i;

const AI_CREDIT_TRAILER_PATTERN =
  /^[\t ]*(co-authored-by|generated-by|assisted-by|ai-assisted-by|ai-generated-by)[\t ]*:[^\r\n]*(claude|codex|chatgpt|gpt|gemini|copilot|openai|anthropic)[^\r\n]*$/gim;

const ZERO_OID_PATTERN = /^0+$/;

function runGit(args, { allowFailure = false } = {}) {
  const result = spawnSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0 && !allowFailure) {
    const detail = result.stderr.trim() || result.stdout.trim();
    throw new Error(`git ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
  }

  return {
    ok: result.status === 0,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

export function findIdentityViolations(identityFields) {
  const violations = [];

  for (const [label, value] of Object.entries(identityFields)) {
    if (MODEL_IDENTITY_PATTERN.test(value)) {
      violations.push(`${label} contains an AI model, product, or vendor identity: ${value}`);
    }
  }

  return violations;
}

export function findMessageViolations(message) {
  const violations = [];
  const matches = message.match(AI_CREDIT_TRAILER_PATTERN) ?? [];

  for (const trailer of matches) {
    violations.push(`commit message contains prohibited AI attribution: ${trailer.trim()}`);
  }

  return violations;
}

function readCurrentIdentity() {
  return {
    author: runGit(["var", "GIT_AUTHOR_IDENT"]).stdout.trim(),
    committer: runGit(["var", "GIT_COMMITTER_IDENT"]).stdout.trim(),
  };
}

function readCommit(commit) {
  const output = runGit([
    "show",
    "-s",
    "--format=%an%x00%ae%x00%cn%x00%ce%x00%B",
    commit,
  ]).stdout;
  const [authorName, authorEmail, committerName, committerEmail, ...messageParts] =
    output.split("\0");

  return {
    authorName,
    authorEmail,
    committerName,
    committerEmail,
    message: messageParts.join("\0"),
  };
}

export function validateCommitData(data) {
  return [
    ...findIdentityViolations({
      "author name": data.authorName,
      "author email": data.authorEmail,
      "committer name": data.committerName,
      "committer email": data.committerEmail,
    }),
    ...findMessageViolations(data.message),
  ];
}

function reportViolations(groups) {
  if (groups.length === 0) {
    return;
  }

  console.error("Git attribution policy violation:");
  for (const { context, violations } of groups) {
    console.error(`\n${context}`);
    for (const violation of violations) {
      console.error(`  - ${violation}`);
    }
  }
  console.error(
    "\nUse the responsible human's Git identity and remove AI-credit trailers. " +
      "Technical references to AI products in ordinary commit prose remain allowed.",
  );
}

function validateCurrentMessage(messagePath) {
  const identity = readCurrentIdentity();
  const message = readFileSync(messagePath, "utf8");
  const violations = [
    ...findIdentityViolations({
      author: identity.author,
      committer: identity.committer,
    }),
    ...findMessageViolations(message),
  ];

  if (violations.length > 0) {
    reportViolations([{ context: "pending commit", violations }]);
    return 1;
  }

  return 0;
}

function validateCommits(commits) {
  const groups = [];

  for (const commit of commits) {
    const violations = validateCommitData(readCommit(commit));
    if (violations.length > 0) {
      groups.push({ context: `commit ${commit}`, violations });
    }
  }

  if (groups.length > 0) {
    reportViolations(groups);
    return 1;
  }

  return 0;
}

function resolveCommit(objectId) {
  const result = runGit(["rev-parse", "--verify", `${objectId}^{commit}`], {
    allowFailure: true,
  });
  return result.ok ? result.stdout.trim() : null;
}

function listCommitsForPush(remoteName, localObjectId, remoteObjectId) {
  const localCommit = resolveCommit(localObjectId);
  if (!localCommit) {
    return [];
  }

  if (!ZERO_OID_PATTERN.test(remoteObjectId)) {
    const remoteCommit = resolveCommit(remoteObjectId);
    if (remoteCommit) {
      return runGit(["rev-list", `${remoteCommit}..${localCommit}`])
        .stdout.trim()
        .split(/\r?\n/)
        .filter(Boolean);
    }
  }

  const remoteSelector = remoteName && !remoteName.includes("://")
    ? `--remotes=${remoteName}`
    : "--remotes";
  return runGit(["rev-list", localCommit, "--not", remoteSelector])
    .stdout.trim()
    .split(/\r?\n/)
    .filter(Boolean);
}

function validatePrePush(remoteName) {
  const input = readFileSync(0, "utf8");
  const commits = new Set();

  for (const line of input.split(/\r?\n/)) {
    if (!line.trim()) {
      continue;
    }

    const [, localObjectId, , remoteObjectId] = line.trim().split(/\s+/);
    if (!localObjectId || !remoteObjectId || ZERO_OID_PATTERN.test(localObjectId)) {
      continue;
    }

    for (const commit of listCommitsForPush(remoteName, localObjectId, remoteObjectId)) {
      commits.add(commit);
    }
  }

  return validateCommits(commits);
}

function usage() {
  console.error(
    "Usage:\n" +
      "  check-git-attribution.mjs current-message <message-file>\n" +
      "  check-git-attribution.mjs commit <commit>...\n" +
      "  check-git-attribution.mjs audit <revision>\n" +
      "  check-git-attribution.mjs pre-push <remote-name> <remote-url>",
  );
}

function main(argv) {
  const [command, ...args] = argv;

  if (command === "current-message" && args.length === 1) {
    return validateCurrentMessage(args[0]);
  }

  if (command === "commit" && args.length > 0) {
    return validateCommits(args);
  }

  if (command === "audit" && args.length === 1) {
    const commits = runGit(["rev-list", args[0]])
      .stdout.trim()
      .split(/\r?\n/)
      .filter(Boolean);
    return validateCommits(commits);
  }

  if (command === "pre-push" && args.length >= 1) {
    return validatePrePush(args[0]);
  }

  usage();
  return 2;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
