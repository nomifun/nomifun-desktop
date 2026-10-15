/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, test } from 'bun:test';
import { runIsolatedDomCase } from '../../../../../../../test/runIsolatedDomCase';

describe('Creative Canvas Agent panel interaction', () => {
  test('recovers hydration, preserves local streaming rows, and reconciles a completed pending turn', async () => {
    await runIsolatedDomCase(
      new URL('./CreativeCanvasAgentPanel.interaction.case.tsx', import.meta.url),
      'creative-canvas-agent-panel-interaction:ok',
      30_000,
    );
  }, 35_000);
});
