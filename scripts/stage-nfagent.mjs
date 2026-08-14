#!/usr/bin/env bun
/**
 * Stage the platform nfagent next to the Tauri resource manifest.
 *
 * nfagent is built by nomifun-net-infra, which is a sibling WSL checkout in
 * the local development layout. The staged copy is intentionally ignored:
 * release artifacts may contain the binary, but the source repository must
 * not gain an unreviewed executable.
 */
import { access, copyFile, mkdir, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const stageDir = path.join(root, 'apps', 'desktop', '.staged-resources', 'nfagent');
const isWindows = process.platform === 'win32';
const sourceName = isWindows ? 'nfagent.exe' : 'nfagent';

function siblingSource() {
  return path.resolve(root, '..', 'nomifun-net-infra', 'bin', sourceName);
}

function wslSource() {
  if (!isWindows) return undefined;
  const distro = process.env.NOMIFUN_WSL_DISTRO || 'Ubuntu';
  const repo = process.env.NOMIFUN_RELAY_REPO_WSL || '/home/rika/code/nomifun-net-infra';
  return `\\\\wsl.localhost\\${distro}${repo.replaceAll('/', '\\')}/bin/${sourceName}`;
}

async function exists(file) {
  try {
    await access(file, constants.R_OK);
    return true;
  } catch {
    return false;
  }
}

const candidates = [
  process.env.NOMIFUN_NFAGENT_SOURCE,
  process.env.NOMIFUN_NFAGENT_PATH,
  siblingSource(),
  wslSource(),
].filter((value) => Boolean(value && value.trim()));

let source;
for (const candidate of candidates) {
  if (await exists(candidate)) {
    source = candidate;
    break;
  }
}
if (!source) {
  console.error(
    `找不到 ${sourceName}。请先构建 nomifun-net-infra，或设置 NOMIFUN_NFAGENT_SOURCE/NOMIFUN_NFAGENT_PATH。`,
  );
  console.error('候选路径：');
  for (const candidate of candidates) console.error(`  ${candidate}`);
  process.exit(1);
}

await rm(stageDir, { recursive: true, force: true });
await mkdir(stageDir, { recursive: true });
const destination = path.join(stageDir, sourceName);
await copyFile(source, destination);
console.log(`已暂存 nfagent: ${source} -> ${destination}`);
