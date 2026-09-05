/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Render smoke test for the model-hub sidebar.
 *
 * `modelHubSections.test.ts` asserts the source contracts (key order, aliases,
 * locale coverage). Those pass just as happily when the grouped render produces
 * nothing: a wrong `React.Fragment` key, a group whose `sections` never map, or a
 * caption swallowing its siblings all leave the source looking correct and the
 * sidebar empty. So this one puts the real page through React with the real
 * locale bundle and checks that every capability actually reaches the DOM as a
 * `tab`, in the designed order, with the captions kept out of the a11y tree.
 *
 * Mirrors the `KnowledgePanel/sessionKnowledgePanelRender.test.tsx` approach
 * (renderToStaticMarkup + a local i18n instance) — there is no testing-library
 * in this repo.
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ModelHubPage from './index';

/**
 * The resizable sider reads a persisted width during render. Keep the stub
 * scoped to each SSR render: the full Bun suite may have installed happy-dom,
 * whose `localStorage` global is a configurable getter without a setter.
 * Assigning to it throws and, more importantly, a module-global replacement
 * would leak this test's storage into unrelated UI files.
 */
function withTestLocalStorage<T>(run: () => T): T {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const store = new Map<string, string>();
  const storage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key),
    clear: () => store.clear(),
    key: (index: number) => [...store.keys()][index] ?? null,
    get length() {
      return store.size;
    },
  } as unknown as Storage;

  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    enumerable: previous?.enumerable ?? false,
    writable: true,
    value: storage,
  });
  try {
    return run();
  } finally {
    if (previous) {
      Object.defineProperty(globalThis, 'localStorage', previous);
    } else {
      delete (globalThis as { localStorage?: Storage }).localStorage;
    }
  }
}

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings } } },
  interpolation: { escapeValue: false },
});

const hub = (zhSettings as unknown as { modelHub: Record<string, string> }).modelHub;

/** The sidebar, top to bottom: one caption per group, one tab per capability. */
const EXPECTED_ORDER = [
  hub.groupAccess,
  hub.sectionModels,
  hub.groupCapability,
  hub.sectionChat,
  hub.sectionRealtime,
  hub.sectionAsr,
  hub.sectionTts,
  hub.sectionVision,
  hub.sectionImage,
  hub.sectionImageEdit,
  hub.sectionVideo,
  hub.sectionEmbedding,
  hub.sectionRerank,
  hub.groupAdvanced,
  hub.sectionFree,
  hub.sectionFailover,
];

const render = (initialEntry: string): string =>
  withTestLocalStorage(() =>
    renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <MemoryRouter initialEntries={[initialEntry]}>
          <ModelHubPage />
        </MemoryRouter>
      </I18nextProvider>
    )
  );

describe('model hub sidebar renders', () => {
  test('every group caption and capability tab reaches the DOM, in order', () => {
    const html = render('/models');
    let cursor = -1;
    for (const label of EXPECTED_ORDER) {
      const at = html.indexOf(`>${label}<`);
      expect(at).toBeGreaterThan(cursor);
      cursor = at;
    }
  });

  test('the sidebar owns exactly one tab per capability', () => {
    const html = render('/models');
    const tabIds = [...html.matchAll(/id="model-hub-tab-([a-z-]+)"/g)].map((m) => m[1]);
    expect(tabIds).toEqual([
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
    ]);
    // A `tablist` may own only `tab` children, so the captions must not be tabs.
    expect((html.match(/role="tab"/g) ?? []).length).toBe(tabIds.length);
  });

  test('the group captions are decoration, not content', () => {
    const html = render('/models');
    for (const caption of [hub.groupAccess, hub.groupCapability, hub.groupAdvanced]) {
      const at = html.indexOf(`>${caption}<`);
      expect(at).toBeGreaterThan(-1);
      // The caption's own element carries aria-hidden.
      const openingTag = html.lastIndexOf('<div', at);
      expect(html.slice(openingTag, at).includes('aria-hidden="true"')).toBe(true);
    }
  });

  test('对话 is selected by default, and a retired key resolves to its heir', () => {
    const fresh = render('/models');
    expect(fresh.includes('aria-labelledby="model-hub-tab-chat"')).toBe(true);

    // `?section=creation` was the 创作能力 host; 图像生成 inherits it.
    const legacy = render('/models?section=creation');
    expect(legacy.includes('aria-labelledby="model-hub-tab-image"')).toBe(true);

    // `?section=global` held the retired global-IDMM tabs; 故障转移 is what is left.
    const retired = render('/models?section=global');
    expect(retired.includes('aria-labelledby="model-hub-tab-failover"')).toBe(true);
  });
});
