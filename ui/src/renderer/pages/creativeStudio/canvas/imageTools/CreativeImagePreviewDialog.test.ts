/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'bun:test';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';

test('image preview interactions with DOM initialized before Arco', async () => {
  // Arco chooses its native event helpers at import time. Isolate from the
  // server-rendered canvas tests so keyboard/drag handlers are actually bound.
  const setup = fileURLToPath(new URL('../../../../../../test/setup-dom.ts', import.meta.url));
  const cases = fileURLToPath(new URL('./CreativeImagePreviewDialog.interaction.case.tsx', import.meta.url));
  await new Promise<void>((resolve, reject) => {
    execFile(process.execPath, ['test', '--preload', setup, cases], { timeout: 15_000 }, (error, stdout, stderr) => {
      if (error) reject(new Error([error.message, stdout, stderr].join('\n')));
      else resolve();
    });
  });
}, 20_000);
