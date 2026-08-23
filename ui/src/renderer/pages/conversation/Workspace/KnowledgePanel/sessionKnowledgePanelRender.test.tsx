/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Render smoke test for the session knowledge panel.
 *
 * The source contracts live in `sessionKnowledgePanel.test.ts`; this one puts the
 * real component through React with the real Arco `Tree` and the real locale
 * bundle, because a `fieldNames` mismatch or a bad root shape produces perfectly
 * valid-looking source and an empty tree. Mirrors the
 * `pages/browser/BrowserPresentation.test.tsx` approach (renderToStaticMarkup +
 * a local i18n instance) — there is no testing-library in this repo.
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId } from '@/common/types/ids';
import zhKnowledge from '@/renderer/services/i18n/locales/zh-CN/knowledge.json';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import SessionKnowledgePanel from './index';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { knowledge: zhKnowledge } } },
  interpolation: { escapeValue: false },
});

const base = (name: string, id: string, rootExists = true): IKnowledgeBase => ({
  knowledge_base_id: id as KnowledgeBaseId,
  name,
  description: '',
  root_path: `/tmp/kb/${id}`,
  managed: true,
  created_at: 0,
  updated_at: 0,
  file_count: 4,
  total_size: 100,
  root_exists: rootExists,
  tags: [],
  kind: 'blank',
});

const render = (bases: IKnowledgeBase[]): string =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <PreviewProvider persistNamespace='verify-knowledge-panel'>
        <SessionKnowledgePanel bases={bases} />
      </PreviewProvider>
    </I18nextProvider>
  );

describe('session knowledge panel renders', () => {
  test('one tree root per mounted knowledge base, in binding order', () => {
    const html = render([base('python基础', 'kb-1'), base('xxxxx 挂载的知识库', 'kb-2')]);

    expect(html.includes('python基础')).toBe(true);
    expect(html.includes('xxxxx 挂载的知识库')).toBe(true);
    expect(html.indexOf('python基础')).toBeLessThan(html.indexOf('xxxxx 挂载的知识库'));
    // Arco rendered actual tree nodes rather than swallowing the data — a
    // fieldNames mismatch shows up here as zero nodes.
    expect(html.includes('arco-tree-node')).toBe(true);
  });

  test('the mounted count and the expand-all control are present', () => {
    const html = render([base('python基础', 'kb-1'), base('产品文档', 'kb-2')]);

    expect(html.includes('已挂载 2 个知识库')).toBe(true);
    // Collapsed on first paint (effects have not run under SSR), so the control
    // offers "expand all" and is reachable by its accessible name.
    expect(html.includes('aria-label="全部展开"')).toBe(true);
  });

  test('a base whose source directory is gone says so, and cannot be expanded', () => {
    const html = render([base('已搬走的库', 'kb-1', false)]);

    expect(html.includes('已搬走的库')).toBe(true);
    // knowledge.mount.rootMissing — NOT the top-level knowledge.rootMissing.
    expect(html.includes('目录不可用')).toBe(true);
    // Nothing is listable, so the expand-all control is disabled rather than
    // firing a request that can only fail.
    expect(html.includes('arco-btn-disabled')).toBe(true);
  });

  test('no mounted bases renders the neutral empty state, not an "is empty" claim', () => {
    const html = render([]);

    expect(html.includes('没有可预览的文档')).toBe(true);
  });
});
