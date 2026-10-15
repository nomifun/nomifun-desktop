/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, test } from 'bun:test';
import { runIsolatedDomCase } from '../../../../../../test/runIsolatedDomCase';

describe('Template Agent draft modal interaction', () => {
  test('clicks a real model option without leaking DOM globals into the UI suite', async () => {
    await runIsolatedDomCase(
      new URL('./TemplateAgentDraftModal.interaction.case.tsx', import.meta.url),
      'template-model-select-interaction:ok',
      15_000,
    );
  }, 20_000);
});
