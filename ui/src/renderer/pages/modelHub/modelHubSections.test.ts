/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';

const src = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

const SECTIONS = [
  'chat',
  'speech',
  'vision',
  'creation',
  'embedding',
  'free',
  'models',
  'global',
] as const;

describe('model hub is a modality-first view', () => {
  test('the eight sections exist in the designed order', () => {
    const start = src.indexOf('const sections: SectionDef[]');
    const list = src.slice(start, src.indexOf('[t]', start));
    const keys = [...list.matchAll(/key: '([a-z]+)'/g)].map((m) => m[1]);
    expect(keys).toEqual([...SECTIONS]);
  });

  test('the default section is 对话, not the provider list', () => {
    expect(src.includes("isSection(param) ? param : 'chat'")).toBe(true);
  });

  test('old bookmarks keep working', () => {
    // `models` / `speech` / `free` / `creation` / `global` were the previous keys
    // and must keep resolving; `agents` keeps its redirect.
    for (const legacy of ['models', 'speech', 'free', 'creation', 'global']) {
      expect(src.includes(`value === '${legacy}'`)).toBe(true);
    }
    expect(src.includes("searchParams.get('section') === 'agents'")).toBe(true);
  });

  test('every section has a label in both locales', () => {
    const labelKey = (s: string) => `section${s[0].toUpperCase()}${s.slice(1)}`;
    for (const locale of [zhSettings, enSettings] as unknown as Record<string, never>[]) {
      const hub = (locale as unknown as { modelHub: Record<string, string> }).modelHub;
      for (const section of SECTIONS) {
        expect(typeof hub[labelKey(section)]).toBe('string');
        expect(hub[labelKey(section)].trim().length > 0).toBe(true);
      }
    }
  });

  test('the provider section is renamed to its narrowed job', () => {
    const zhHub = (zhSettings as unknown as { modelHub: Record<string, string> }).modelHub;
    expect(zhHub.sectionModels).toBe('供应商与密钥');
    const zhProvider = (
      zhSettings as unknown as { modelHub: { provider: Record<string, string> } }
    ).modelHub.provider;
    expect(typeof zhProvider.scopeNote).toBe('string');
    const enProvider = (
      enSettings as unknown as { modelHub: { provider: Record<string, string> } }
    ).modelHub.provider;
    expect(typeof enProvider.scopeNote).toBe('string');
  });
});
