/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('skill localization integration', () => {
  test('the Agent Workbench reads display metadata from the canonical Skill catalog', () => {
    const editor = readSource(new URL('../../agentSettings/AgentPresetEditor.tsx', import.meta.url));
    expect(editor.includes('catalog.skills.map((skill)')).toBe(true);
    expect(editor.includes('skill.display_name')).toBe(true);
    expect(editor.includes('skill.description')).toBe(true);
  });
});
