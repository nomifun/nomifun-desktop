import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { spawnSync } from "node:child_process";

function runGit(args) {
  const result = spawnSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    const detail = result.stderr.trim() || result.stdout.trim();
    throw new Error(`git ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
  }

  return result.stdout.trim();
}

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const expectedRoot = resolve(scriptDirectory, "..");
const repositoryRoot = resolve(runGit(["rev-parse", "--show-toplevel"]));

if (repositoryRoot.toLowerCase() !== expectedRoot.toLowerCase()) {
  throw new Error(
    `Refusing to configure a different repository: expected ${expectedRoot}, got ${repositoryRoot}`,
  );
}

runGit(["config", "--local", "core.hooksPath", ".githooks"]);
const configuredPath = runGit(["config", "--local", "--get", "core.hooksPath"]);

if (configuredPath !== ".githooks") {
  throw new Error(`Unexpected repository hook path: ${configuredPath}`);
}

console.log(`Enabled repository-local Git hooks for ${repositoryRoot}`);
console.log("Global Git configuration was not changed.");
