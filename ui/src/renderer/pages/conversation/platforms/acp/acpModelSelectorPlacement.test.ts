/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('ACP conversation model selector placement', () => {
  test('renders the model selector immediately before the permission selector in the sendbox tools', () => {
    const source = readSource(new URL('./AcpSendBox.tsx', import.meta.url));
    const rightToolsIndex = source.indexOf('rightTools={');
    const modelIndex = source.indexOf('<AcpModelSelector', rightToolsIndex);
    const permissionIndex = source.indexOf('<AgentModeSelector', rightToolsIndex);

    expect(rightToolsIndex).toBeGreaterThan(-1);
    expect(modelIndex).toBeGreaterThan(rightToolsIndex);
    expect(permissionIndex).toBeGreaterThan(modelIndex);
  });

  test('does not render ACP model switching in the conversation header', () => {
    const source = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));

    expect(source.includes('<AcpModelSelector')).toBe(false);
  });
});
