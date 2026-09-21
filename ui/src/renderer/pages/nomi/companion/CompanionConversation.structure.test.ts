/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');

describe('CompanionConversation structure', () => {
  test('uses the route-level titlebar workspace toggle instead of self-contained panel toggles', () => {
    const source = readSource('./CompanionConversation.tsx');

    expect(source.includes('selfContainedWorkspaceToggle')).toBe(false);
    expect(source.includes('ExecutionConversationLayout')).toBe(false);
  });

  test('uses the fixed Companion Agent and keeps the composer chat-only', () => {
    const conversation = readSource('./CompanionConversation.tsx');
    const models = readSource('../workspace/tabs/OverviewTab/ModelsSection.tsx');

    expect(conversation.includes('<CompanionAgentIndicator')).toBe(true);
    expect(conversation.includes('creationEnabled={false}')).toBe(true);
    expect(conversation.includes('ProductAgentBindingSelect')).toBe(false);
    expect(models.includes('<CompanionAgentIndicator')).toBe(true);
    expect(models.includes('ProductAgentBindingSelect')).toBe(false);
  });
});
