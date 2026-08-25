/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { PromptLibrarySurface } from './PromptLibrarySurface';
import type { PromptLibraryItem, PromptLibrarySelection } from './types';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

const ITEM: PromptLibraryItem = {
  id: 'prompt-copy',
  source: 'preset',
  title: '复制测试',
  description: '测试复制完整提示词',
  prompt: '保留完整的提示词内容，不使用卡片预览文本。',
  category: '测试',
  tags: ['复制'],
  knowledgeBaseIds: [],
  coverUrl: null,
  preview: null,
  sourceUrl: null,
  license: null,
  licenseUrl: null,
  createdAt: null,
  updatedAt: null,
  savedToAssets: false,
};

afterEach(() => {
  cleanup();
});

describe('PromptLibrarySurface copy interaction', () => {
  test('passes the complete prompt selection to the copy action', () => {
    const copied: PromptLibrarySelection[] = [];
    const { container } = render(
      <I18nextProvider i18n={testI18n}>
        <PromptLibrarySurface
          variant='sidebar'
          items={[ITEM]}
          onCopy={(selection) => copied.push(selection)}
        />
      </I18nextProvider>
    );

    const copyButton = container.querySelector<HTMLButtonElement>(
      'button[data-prompt-library-action="copy"]'
    );
    expect(copyButton).not.toBeNull();
    fireEvent.click(copyButton!);

    expect(copied).toEqual([
      {
        id: ITEM.id,
        source: ITEM.source,
        title: ITEM.title,
        prompt: ITEM.prompt,
        category: ITEM.category,
        tags: ITEM.tags,
        knowledgeBaseIds: ITEM.knowledgeBaseIds,
        coverUrl: ITEM.coverUrl,
        sourceUrl: ITEM.sourceUrl,
        license: ITEM.license,
        licenseUrl: ITEM.licenseUrl,
      },
    ]);
  });
});
