/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('SkillsHubSettings agent skill migration entry', () => {
  test('exposes an Agent Skills import action backed by external source detection', () => {
    const source = readSource(new URL('./SkillsHubSettings.tsx', import.meta.url));

    expect(source.includes('AgentSkillImportDrawer')).toBe(true);
    expect(source.includes("data-testid='btn-import-agent-skills'")).toBe(true);
    expect(source.includes('detectAndCountExternalSkills')).toBe(true);
    expect(source.includes('setAgentImportVisible(true)')).toBe(true);
  });
});
