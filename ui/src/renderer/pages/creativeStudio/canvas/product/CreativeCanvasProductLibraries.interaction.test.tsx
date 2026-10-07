/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type {
  PromptLibraryItem,
  PromptLibraryPort,
  PromptLibrarySelection,
} from '../../prompts';
import { CreativeCanvasProductPromptLibrary } from './CreativeCanvasProductLibraries';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const ITEM: PromptLibraryItem = {
  id: 'canvas-prompt',
  source: 'catalog',
  title: '画布提示词',
  description: '保留完整的提示词内容',
  prompt: '一段要复制到画布工作流的完整提示词。',
  category: '画布',
  tags: ['构图'],
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

const port: PromptLibraryPort = {
  list: async () => [ITEM],
};

afterEach(cleanup);

describe('CreativeCanvasProductPromptLibrary', () => {
  test('uses the shared visual picker and copies only after its overlay action', async () => {
    const selectedIds: string[] = [];
    const copied: PromptLibrarySelection[] = [];
    const { container } = render(
      <I18nextProvider i18n={testI18n}>
        <CreativeCanvasProductPromptLibrary
          locale='zh-CN'
          port={port}
          onSelect={(id) => selectedIds.push(id)}
          onCopy={(selection) => copied.push(selection)}
        />
      </I18nextProvider>
    );

    await waitFor(() => {
      expect(
        container.querySelector('button[data-prompt-card-action="activate"]')
      ).not.toBeNull();
    });

    const activate = container.querySelector<HTMLButtonElement>(
      'button[data-prompt-card-action="activate"]'
    );
    const apply = container.querySelector<HTMLButtonElement>(
      'button[data-prompt-card-action="apply"]'
    );
    expect(container.querySelector('header')).toBeNull();
    expect(apply?.textContent).toContain('复制提示词');

    fireEvent.click(activate!);
    expect(selectedIds).toEqual([]);
    expect(copied).toEqual([]);

    fireEvent.click(apply!);
    expect(selectedIds).toEqual([ITEM.id]);
    expect(copied[0]?.prompt).toBe(ITEM.prompt);
  });
});
