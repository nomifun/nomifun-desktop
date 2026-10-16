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

  test('uses the companion-owned Agent selector and keeps the composer chat-only', () => {
    const conversation = readSource('./CompanionConversation.tsx');
    const models = readSource('../workspace/tabs/OverviewTab/ModelsSection.tsx');

    expect(conversation.includes('<ProductAgentBindingSelect')).toBe(true);
    expect(conversation.includes('targetKind={role}')).toBe(true);
    expect(conversation.includes('conversationId={conversation.id}')).toBe(true);
    expect(conversation.includes('onChanged={refreshSession}')).toBe(true);
    expect(conversation.includes('creationEnabled={false}')).toBe(true);
    expect(conversation.includes("creationTasksEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('creation.media') === true}")).toBe(true);
    expect(models.includes('<ProductAgentBindingSelect')).toBe(true);
    expect(models.includes('targetId={profile.companion_id}')).toBe(true);
  });
});
