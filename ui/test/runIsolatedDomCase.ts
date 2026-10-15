/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const readStream = (stream: NodeJS.ReadableStream): Promise<string> =>
  new Promise((resolve, reject) => {
    let output = '';
    stream.setEncoding('utf8');
    stream.on('data', (chunk: string) => { output += chunk; });
    stream.once('end', () => resolve(output));
    stream.once('error', reject);
  });

/** Keep DOM-sensitive interactions isolated while checking their full result. */
export async function runIsolatedDomCase(caseUrl: URL, successMarker: string, timeoutMs: number) {
  const child = spawn(process.execPath, [
    '--preload', fileURLToPath(new URL('./setup-assets.mjs', import.meta.url)),
    '--preload', fileURLToPath(new URL('./setup-dom.ts', import.meta.url)),
    fileURLToPath(caseUrl),
  ], {
    cwd: fileURLToPath(new URL('../', import.meta.url)),
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  let timedOut = false;
  const killTimer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
  try {
    const [result, stdout, stderr] = await Promise.all([
      new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve, reject) => {
        child.once('error', reject);
        child.once('exit', (code, signal) => resolve({ code, signal }));
      }),
      readStream(child.stdout),
      readStream(child.stderr),
    ]);
    if (timedOut || result.code !== 0 || result.signal !== null ||
        !stdout.includes(successMarker) || stderr.trim() !== '') {
      throw new Error([
        `Isolated DOM interaction failed (timeout=${String(timedOut)}, code=${String(result.code)}, signal=${String(result.signal)}).`,
        stdout.trim(),
        stderr.trim(),
      ].filter(Boolean).join('\n'));
    }
  } finally {
    clearTimeout(killTimer);
  }
}
