#!/usr/bin/env node
// A source transfer manifest, not a test report or a 99% reliability claim.
// Includes in-scope untracked files that a plain `git diff` would lose.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, lstatSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = realpathSync(fileURLToPath(new URL('../../', import.meta.url)));
const manifestPath = 'docs/specs/2026-09-25-agent-reliability/handoff-manifest.json';
const roots = new Set(['Cargo.toml', 'Cargo.lock', 'package.json', 'README.md', 'README.zh-CN.md', 'scripts/scripts.json', 'ui/src/common/protocolBindings/TurnStopReason.ts']);
const crates = new Set([
  'nomifun-agent-contracts', 'nomifun-agent-domain-wave5', 'nomifun-agent-runtime',
  'nomifun-agent-session', 'nomifun-ai-agent', 'nomifun-app', 'nomifun-chat-model-broker',
  'nomifun-db', 'nomifun-engine-core', 'nomifun-model-invoke',
  'nomifun-channel',
]);

function allowed(path) {
  if (typeof path !== 'string' || path.includes('\\') || isAbsolute(path)
    || path.split('/').some((part) => !part || part === '.' || part === '..')
    || path.includes(':') || path === manifestPath) return false;
  if (roots.has(path)) return true;
  if (path.startsWith('docs/specs/2026-09-25-agent-reliability/')) return /\.md$/.test(path);
  if (/^scripts\/validation\/(agent-reliability-[\w.-]+|probe-stepfun-tool-schema(?:\.test)?|run-nomi-core-live-provider-smoke)\.mjs$/.test(path)) return true;
  const parts = path.split('/');
  return parts[0] === 'crates' && parts[1] === 'backend' && crates.has(parts[2])
    && /\.(?:rs|toml|sql|json)$/.test(path)
    && !parts.some((part) => part.startsWith('.') || /^(?:target|build\.noindex|node_modules|data)$/.test(part));
}

function withinRoot(path) {
  const offset = relative(root, path);
  if (offset === '..' || offset.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(offset)) {
    throw new Error('A handoff path resolves outside the workspace');
  }
  return path;
}

function describe(path) {
  if (!allowed(path)) throw new Error(`Out-of-scope handoff path: ${path}`);
  const target = withinRoot(resolve(root, path));
  if (!existsSync(target)) return { path, state: 'deleted' };
  const stat = lstatSync(target);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 8 * 1024 * 1024) {
    throw new Error(`Handoff accepts bounded regular source files only: ${path}`);
  }
  withinRoot(realpathSync(target));
  return { path, state: 'present', bytes: stat.size,
    sha256: createHash('sha256').update(readFileSync(target)).digest('hex') };
}

function git(args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 });
}

function writeManifest() {
  const changed = git(['diff', '--name-only', '-z', 'HEAD', '--']).split('\0').filter(Boolean);
  const untracked = git(['ls-files', '--others', '--exclude-standard', '-z']).split('\0').filter(Boolean);
  const paths = [...new Set([...changed, ...untracked])].filter(allowed).sort();
  const manifest = {
    version: 1,
    purpose: 'source-transfer-integrity-only-not-test-evidence',
    generated_at: new Date().toISOString(),
    base_commit: git(['rev-parse', 'HEAD']).trim(),
    test_status: 'deferred-for-staged-validation-see-DEVELOPMENT-HANDOFF.zh.md',
    files: paths.map(describe),
  };
  const output = withinRoot(resolve(root, manifestPath));
  withinRoot(realpathSync(dirname(output)));
  if (existsSync(output) && lstatSync(output).isSymbolicLink()) throw new Error('Manifest output cannot be a symbolic link');
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  console.log(`Wrote ${manifestPath}: ${manifest.files.length} in-scope source files, including untracked files.`);
  console.log('Only paths, sizes and hashes were exported; no file bodies, environment values, commits or test-success assertions.');
}

function checkManifest() {
  const source = withinRoot(realpathSync(resolve(root, manifestPath)));
  if (lstatSync(source).size > 1024 * 1024) throw new Error('Handoff manifest exceeds its size limit');
  const manifest = JSON.parse(readFileSync(source, 'utf8'));
  if (manifest.version !== 1 || manifest.purpose !== 'source-transfer-integrity-only-not-test-evidence'
    || !/^[a-f0-9]{40}(?:[a-f0-9]{24})?$/.test(manifest.base_commit)
    || !Array.isArray(manifest.files) || !manifest.files.length || manifest.files.length > 2048) {
    throw new Error('Invalid handoff manifest');
  }
  if (git(['rev-parse', 'HEAD']).trim() !== manifest.base_commit) {
    throw new Error('Handoff Git baseline differs. Compare/integrate local work explicitly; do not reset it or regenerate the manifest to hide the mismatch.');
  }
  const seen = new Set();
  const mismatches = [];
  for (const expected of manifest.files) {
    if (!expected || !allowed(expected.path) || seen.has(expected.path)
      || !['present', 'deleted'].includes(expected.state)
      || (expected.state === 'present' && (!/^[a-f0-9]{64}$/.test(expected.sha256)
        || !Number.isSafeInteger(expected.bytes) || expected.bytes < 0))) {
      throw new Error('Invalid or repeated source entry in handoff manifest');
    }
    seen.add(expected.path);
    const actual = describe(expected.path);
    if (actual.state !== expected.state || actual.bytes !== expected.bytes || actual.sha256 !== expected.sha256) {
      mismatches.push(expected.path);
    }
  }
  if (mismatches.length) throw new Error(`Handoff source mismatch (do not overwrite local edits blindly):\n${mismatches.join('\n')}`);
  console.log(`Handoff source hashes match for ${manifest.files.length} files. This is transfer integrity, not test acceptance.`);
}

try {
  const args = process.argv.slice(2);
  if (args.length !== 1 || !['--write', '--check'].includes(args[0])) {
    throw new Error('Usage: node scripts/validation/agent-reliability-handoff.mjs --write|--check');
  }
  if (args[0] === '--write') writeManifest(); else checkManifest();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
