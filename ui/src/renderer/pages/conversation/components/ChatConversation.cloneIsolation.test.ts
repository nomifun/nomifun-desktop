/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./ChatConversation.tsx', import.meta.url), 'utf8');

describe('new conversation workspace isolation', () => {
  test('removes source workspace and resume state before invoking clone', () => {
    for (const field of [
      'workspace: _sourceWorkspace',
      'temp_workspace_id: _sourceTempWorkspaceId',
      'acp_session_id: _sourceAcpSessionId',
      'current_mode_id: _sourceCurrentModeId',
      'current_model_id: _sourceCurrentModelId',
      'runtimeValidation: _sourceRuntimeValidation',
      'sessionKey: _sourceSessionKey',
    ]) {
      expect(source.includes(field)).toBe(true);
    }
    expect(source.includes('extra: freshExtra')).toBe(true);
    expect(source.includes('...source.extra')).toBe(false);
  });
});
