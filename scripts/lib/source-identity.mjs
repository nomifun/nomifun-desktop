import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { join } from 'node:path';

const git = (repoRoot, args) => new Promise((resolve, reject) => {
  const child = spawn('git', ['-C', repoRoot, ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
  const chunks = [];
  const errors = [];
  child.stdout.on('data', (chunk) => chunks.push(chunk));
  child.stderr.on('data', (chunk) => errors.push(chunk));
  child.on('error', reject);
  child.on('close', (code, signal) => {
    if (code === 0) resolve(Buffer.concat(chunks));
    else reject(new Error(`git ${args.join(' ')} failed: code=${code} signal=${signal} ${Buffer.concat(errors).toString('utf8').trim()}`));
  });
});

const gitStream = (repoRoot, args, hash) => new Promise((resolve, reject) => {
  const child = spawn('git', ['-C', repoRoot, ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
  const errors = [];
  child.stdout.on('data', (chunk) => hash.update(chunk));
  child.stderr.on('data', (chunk) => errors.push(chunk));
  child.on('error', reject);
  child.on('close', (code, signal) => {
    if (code === 0) resolve(hash.digest('hex'));
    else reject(new Error(`git ${args.join(' ')} failed: code=${code} signal=${signal} ${Buffer.concat(errors).toString('utf8').trim()}`));
  });
});

const fileSha256 = (path) => new Promise((resolve, reject) => {
  const hash = createHash('sha256');
  createReadStream(path).on('error', reject).on('data', (chunk) => hash.update(chunk)).on('end', () => resolve(hash.digest('hex')));
});

export async function sourceIdentity(repoRoot) {
  try {
    const head = (await git(repoRoot, ['rev-parse', 'HEAD'])).toString('utf8').trim();
    const diff_sha256 = await gitStream(repoRoot, ['diff', '--no-ext-diff', '--no-color', '--binary', 'HEAD'], createHash('sha256'));
    const names = (await git(repoRoot, ['ls-files', '--others', '--exclude-standard', '-z'])).toString('utf8').split('\0').filter(Boolean);
    const untracked = {};
    for (const name of names) {
      try { untracked[name] = await fileSha256(join(repoRoot, name)); }
      catch (error) { if (error.code === 'ENOENT') continue; throw error; }
    }
    return { head, diff_sha256, untracked };
  } catch (error) {
    return { error: String(error?.message ?? error) };
  }
}
