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

/** The sidebar order, top to bottom, across all three groups. */
const SECTIONS = [
  'models',
  'chat',
  'realtime',
  'asr',
  'tts',
  'vision',
  'image',
  'image-edit',
  'video',
  'embedding',
  'rerank',
  'free',
  'failover',
] as const;

const GROUPS = [
  { key: 'access', sections: ['models'] },
  {
    key: 'capability',
    sections: ['chat', 'realtime', 'asr', 'tts', 'vision', 'image', 'image-edit', 'video', 'embedding', 'rerank'],
  },
  { key: 'advanced', sections: ['free', 'failover'] },
] as const;

const hubOf = (locale: unknown): Record<string, string> =>
  (locale as { modelHub: Record<string, string> }).modelHub;

describe('model hub is a capability-first view', () => {
  test('all nine tasks plus the vision trait projection exist independently', () => {
    const start = src.indexOf('const SECTION_KEYS');
    const list = src.slice(start, src.indexOf('];', start));
    const keys = [...list.matchAll(/'([a-z-]+)'/g)].map((m) => m[1]);
    expect(keys).toEqual([...SECTIONS]);
  });

  test('providers lead the sidebar — a provider is the source of every model', () => {
    expect(SECTIONS[0]).toBe('models');
    const groupStart = src.indexOf('const SECTION_GROUPS');
    const groupSrc = src.slice(groupStart, src.indexOf('const FLAT_SECTIONS', groupStart));
    // Group order, and the section order inside each group, both matter.
    const groupKeys = [...groupSrc.matchAll(/^ {4}key: '([a-z]+)',$/gm)].map((m) => m[1]);
    expect(groupKeys).toEqual(GROUPS.map((g) => g.key));
    const sectionKeys = [...groupSrc.matchAll(/^ {8}key: '([a-z-]+)',$/gm)].map((m) => m[1]);
    expect(sectionKeys).toEqual([...SECTIONS]);
  });

  test('the free-model section sits in the last group, below the capabilities', () => {
    // Same rule the provider groups inside every capability section follow:
    // NomiFun-managed models rank below what the user configured.
    const advanced = GROUPS[GROUPS.length - 1].sections as readonly string[];
    expect(advanced.includes('free')).toBe(true);
    expect(SECTIONS.indexOf('free')).toBeGreaterThan(SECTIONS.indexOf('embedding'));
  });

  test('the default section is 对话, not the provider list', () => {
    expect(src.includes("resolveSection(searchParams.get('section')) ?? 'chat'")).toBe(true);
  });

  test('old bookmarks keep working', () => {
    // `speech` and `creation` were HOSTS holding several categories each; they
    // now have one section per category, so an old link resolves to the first.
    expect(src.includes("speech: 'asr'")).toBe(true);
    expect(src.includes("creation: 'image'")).toBe(true);
    // `global` held the IDMM defaults + the failover queue + decision activity.
    // The global-IDMM concept is gone entirely, so the section is named after
    // what actually remains.
    expect(src.includes("global: 'failover'")).toBe(true);
    // `models` / `free` are still real keys, so they resolve as-is.
    for (const stillReal of ['models', 'free'] as const) {
      expect(SECTIONS.includes(stillReal)).toBe(true);
    }
    expect(src.includes("searchParams.get('section') === 'agents'")).toBe(true);
  });

  test('no global-IDMM surface survives in the hub', () => {
    // 全局 IDMM 配置 / 决策活动 were removed together with the global-backup
    // concept they configured; the wrapper that hosted them is gone too.
    expect(src.includes('GlobalModelConfig')).toBe(false);
    expect(src.includes('IdmmActivityContent')).toBe(false);
    expect(src.includes('<ModelFailoverContent />')).toBe(true);
  });

  test('every section and group has a label in both locales', () => {
    const labelKey = (s: string) =>
      `section${s.split('-').map((part) => `${part[0].toUpperCase()}${part.slice(1)}`).join('')}`;
    const groupKey = (g: string) => `group${g[0].toUpperCase()}${g.slice(1)}`;
    for (const locale of [zhSettings, enSettings]) {
      const hub = hubOf(locale);
      for (const key of [...SECTIONS.map(labelKey), ...GROUPS.map((g) => groupKey(g.key))]) {
        expect(typeof hub[key]).toBe('string');
        expect(hub[key].trim().length > 0).toBe(true);
      }
    }
  });

  test('the retired host labels are gone from both locales', () => {
    for (const locale of [zhSettings, enSettings]) {
      const hub = hubOf(locale);
      expect(hub.sectionSpeech).toBeUndefined();
      expect(hub.sectionCreation).toBeUndefined();
    }
  });

  test('the provider section is renamed to its narrowed job', () => {
    const zhHub = hubOf(zhSettings);
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

  test('the group captions stay out of the a11y tree', () => {
    // `tablist` may own only `tab` children. The captions are decoration; the
    // tabs already carry their own labels and position.
    expect(src.includes("aria-hidden='true'")).toBe(true);
    expect(src.includes("role='tablist'")).toBe(true);
  });
});
