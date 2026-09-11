/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import type { ImageWorkbenchResult } from './types';

const { default: ImageWorkbenchResults } = await import('./ImageWorkbenchResults');

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { creativeStudio: zh } } },
});

const result: ImageWorkbenchResult = {
  id: 'result-detail',
  taskId: 'task-detail',
  status: 'succeeded',
  prompt: '雨夜霓虹街道，电影感构图，真实光影',
  model: { providerId: 'provider-a', model: 'image-model' },
  modelLabel: 'Provider A · Image Model',
  createdAtLabel: '2026-09-11 10:20',
  durationLabel: '2.8 s',
  deletable: true,
  outputs: [
    {
      assetId: 'asset-detail',
      imageUrl: 'https://media.invalid/detail.png',
      alt: '雨夜霓虹街道',
      width: 1024,
      height: 1536,
      sizeLabel: '2.1 MB',
    },
  ],
};

afterEach(() => {
  cleanup();
  document.getElementById('creative-studio-portal-root')?.remove();
});

const renderResults = (onSelectionChange: (ids: string[]) => void = () => undefined) => {
  const portal = document.createElement('div');
  portal.id = 'creative-studio-portal-root';
  document.body.appendChild(portal);
  const rendered = render(
    <I18nextProvider i18n={i18n}>
      <ImageWorkbenchResults
        results={[result]}
        selectedResultIds={[]}
        task={{ state: 'succeeded', pendingCount: 0 }}
        onSelectionChange={onSelectionChange}
        onDeleteResult={() => undefined}
        onDeleteSelected={() => undefined}
      />
    </I18nextProvider>
  );
  return { ...rendered, portal };
};

describe('ImageWorkbenchResults interactions', () => {
  test('opens the complete artwork details when a card is clicked', async () => {
    const { container, portal } = renderResults();
    fireEvent.click(container.querySelector('[data-image-result-state="succeeded"]')!);

    await waitFor(() => {
      expect(within(portal).getByText('作品详情')).not.toBeNull();
    });
    expect(within(portal).getByText(result.prompt)).not.toBeNull();
    expect(within(portal).getByText('1024 × 1536 · 2.1 MB')).not.toBeNull();
    expect(within(portal).getByText('provider-a')).not.toBeNull();
    expect(within(portal).getByText('image-model')).not.toBeNull();
    expect(within(portal).getByRole('button', { name: '复制提示词' })).not.toBeNull();
  });

  test('keeps selection controls independent from card detail opening', () => {
    const changes: string[][] = [];
    const { getByRole, portal } = renderResults((ids) => changes.push(ids));
    fireEvent.click(getByRole('checkbox', { name: '选择结果 result-detail' }));

    expect(changes).toEqual([['result-detail']]);
    expect(within(portal).queryByText('作品详情')).toBeNull();
  });
});
