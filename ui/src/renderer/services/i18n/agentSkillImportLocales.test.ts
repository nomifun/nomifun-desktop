/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enSettings from './locales/en-US/settings.json';
import zhSettings from './locales/zh-CN/settings.json';

const REQUIRED_AGENT_SKILL_IMPORT_KEYS = [
  'title',
  'shortAction',
  'description',
  'sources',
  'detectError',
  'detectErrorDetailed',
  'importError',
  'importErrorDetailed',
  'librarySuccess',
  'assistantSuccess',
  'selectionSummary',
  'addToAssistant',
  'importSelected',
  'selectAll',
  'count',
  'alreadyImported',
  'empty',
] as const;

describe('agent skill import locale coverage', () => {
  const assertLocaleCoverage = (settings: { agentSkillImport?: Record<string, string> }) => {
    const agentSkillImport = settings.agentSkillImport as Record<string, string> | undefined;

    expect(agentSkillImport).toBeDefined();
    for (const key of REQUIRED_AGENT_SKILL_IMPORT_KEYS) {
      expect(agentSkillImport?.[key]?.trim()).toBeTruthy();
    }
  };

  test('en-US defines every Agent Skill import string', () => {
    assertLocaleCoverage(enSettings);
  });

  test('zh-CN defines every Agent Skill import string', () => {
    assertLocaleCoverage(zhSettings);
  });
});
