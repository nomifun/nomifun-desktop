/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Assistant editor agent skill migration integration', () => {
  test('offers importing external Agent Skills directly into the edited assistant', () => {
    const drawer = readSource(new URL('./AssistantEditDrawer.tsx', import.meta.url));
    const editorHook = readSource(new URL('../../../hooks/assistant/useAssistantEditor.ts', import.meta.url));
    const host = readSource(new URL('./index.tsx', import.meta.url));

    expect(drawer.includes('AgentSkillImportDrawer')).toBe(true);
    expect(drawer.includes("data-testid='btn-import-agent-skills-to-assistant'")).toBe(true);
    expect(drawer.includes('onImportAgentSkills')).toBe(true);

    expect(editorHook.includes('handleImportAgentSkills')).toBe(true);
    expect(editorHook.includes('importSkillWithSymlink.invoke')).toBe(true);
    expect(editorHook.includes('mergeImportedSkillNames')).toBe(true);

    expect(host.includes('onImportAgentSkills={editor.handleImportAgentSkills}')).toBe(true);
  });
});
