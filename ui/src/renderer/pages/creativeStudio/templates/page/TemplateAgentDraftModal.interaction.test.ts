/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { describe, test } from 'bun:test';

const SUCCESS_MARKER = 'template-model-select-interaction:ok';

const readStream = (stream: NodeJS.ReadableStream): Promise<string> =>
  new Promise((resolve, reject) => {
    let output = '';
    stream.setEncoding('utf8');
    stream.on('data', (chunk: string) => {
      output += chunk;
    });
    stream.once('end', () => resolve(output));
    stream.once('error', reject);
  });

describe('Template Agent draft modal interaction', () => {
  test('clicks a real model option without leaking DOM globals into the UI suite', async () => {
    const uiRoot = fileURLToPath(new URL('../../../../../../', import.meta.url));
    const setupPath = fileURLToPath(
      new URL('../../../../../../test/setup-dom.ts', import.meta.url)
    );
    const casePath = fileURLToPath(
      new URL('./TemplateAgentDraftModal.interaction.case.tsx', import.meta.url)
    );
    const child = spawn(process.execPath, ['--preload', setupPath, casePath], {
      cwd: uiRoot,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let timedOut = false;
    const killTimer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, 15_000);
    const stdoutPromise = readStream(child.stdout);
    const stderrPromise = readStream(child.stderr);
    let result: { code: number | null; signal: NodeJS.Signals | null };
    let stdout: string;
    let stderr: string;
    try {
      result = await new Promise<{ code: number | null; signal: NodeJS.Signals | null }>(
        (resolve, reject) => {
          child.once('error', reject);
          child.once('exit', (code, signal) => resolve({ code, signal }));
        }
      );
      [stdout, stderr] = await Promise.all([stdoutPromise, stderrPromise]);
    } finally {
      clearTimeout(killTimer);
    }

    if (
      timedOut ||
      result.code !== 0 ||
      result.signal !== null ||
      !stdout.includes(SUCCESS_MARKER) ||
      stderr.trim() !== ''
    ) {
      throw new Error(
        [
          `Isolated DOM interaction failed (timeout=${String(timedOut)}, code=${String(result.code)}, signal=${String(result.signal)}).`,
          stdout.trim(),
          stderr.trim(),
        ]
          .filter(Boolean)
          .join('\n')
      );
    }
  }, 20_000);
});
