/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import CreativeCanvasReferenceList, { type CreativeCanvasImageComposerReference } from './CreativeCanvasReferenceList';

afterEach(cleanup);
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { creativeStudio: zh } } } });
const references: CreativeCanvasImageComposerReference[] = [
  { nodeId: 'base', assetId: 'own', connectionId: null, base: true, label: '当前图片', ordinal: 1 },
  { nodeId: 'image', assetId: 'asset', connectionId: 'image-edge', base: false, label: '猫咪', ordinal: 2 },
  { nodeId: 'text', assetId: null, connectionId: 'text-edge', base: false, kind: 'text', textContent: '', label: '文本1', mentionLabel: '文本1', ordinal: 1, disabledReason: '文本为空' },
];
const wrap = (content: React.ReactNode) => <I18nextProvider i18n={i18n}>{content}</I18nextProvider>;

describe('CreativeCanvasReferenceList', () => {
  test('falls back from a broken thumbnail to the original without changing reference actions', () => {
    const activations: string[] = [];
    const reference = {
      ...references[1],
      thumbnailUrl: '/reference-thumbnail.jpg',
      originalUrl: '/reference-original.png',
    };
    const { getByRole } = render(wrap(<CreativeCanvasReferenceList
      references={[reference]}
      onActivate={(id) => activations.push(id)}
    />));
    const button = getByRole('button', { name: '定位参考 猫咪' });
    const image = button.querySelector('img')!;
    expect(image.getAttribute('src')).toBe(reference.thumbnailUrl);

    fireEvent.error(image);
    expect(button.querySelector('img')?.getAttribute('src')).toBe(reference.originalUrl);
    fireEvent.click(button);
    expect(activations).toEqual([reference.nodeId]);
  });

  test('selects valid and unavailable references in one batch while preserving the base image', () => {
    const calls: string[][] = [];
    const activations: string[] = [];
    const { getByRole, queryByRole } = render(wrap(<CreativeCanvasReferenceList
      references={references}
      onActivate={(id) => activations.push(id)}
      onDisconnectMany={(ids) => calls.push([...ids])}
    />));
    fireEvent.click(getByRole('button', { name: '批量管理' }));
    expect((getByRole('button', { name: '断开所选' }) as HTMLButtonElement).disabled).toBe(true);
    expect((getByRole('button', { name: '选择参考 当前图片' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(getByRole('button', { name: '选择参考 猫咪' }));
    expect(getByRole('button', { name: '选择参考 猫咪' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(getByRole('button', { name: '选择参考 猫咪' }));
    expect(getByRole('button', { name: '选择参考 猫咪' }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(getByRole('button', { name: '全选连接' }));
    expect(getByRole('button', { name: '选择参考 文本1' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(getByRole('button', { name: '断开所选' }));
    expect(calls).toEqual([['image-edge', 'text-edge']]);
    expect(activations).toEqual([]);
    expect(queryByRole('group', { name: '批量管理' })).toBeNull();
    fireEvent.click(getByRole('button', { name: '定位参考 猫咪' }));
    expect(activations).toEqual(['image']);
  });

  test('prunes disconnected selections, respects disabled state and cancels without deleting', () => {
    const calls: string[][] = [];
    const view = (items = references, disabled = false) => wrap(<CreativeCanvasReferenceList
      references={items} disabled={disabled} onDisconnectMany={(ids) => calls.push([...ids])}
    />);
    const { getByRole, queryByRole, rerender } = render(view());
    fireEvent.click(getByRole('button', { name: '批量管理' }));
    fireEvent.click(getByRole('button', { name: '全选连接' }));
    rerender(view([references[0], references[2]]));
    rerender(view());
    expect(getByRole('button', { name: '选择参考 猫咪' }).getAttribute('aria-pressed')).toBe('false');
    rerender(view(references, true));
    fireEvent.click(getByRole('button', { name: '断开所选' }));
    expect(calls).toEqual([]);
    rerender(view());
    fireEvent.keyDown(getByRole('button', { name: '选择参考 文本1' }), { key: 'Escape' });
    expect(queryByRole('group', { name: '批量管理' })).toBeNull();
    expect(calls).toEqual([]);
    fireEvent.click(getByRole('button', { name: '批量管理' }));
    expect((getByRole('button', { name: '断开所选' }) as HTMLButtonElement).disabled).toBe(true);
    rerender(view([references[0]]));
    expect(queryByRole('button', { name: '批量管理' })).toBeNull();
    expect(queryByRole('button', { name: '断开所选' })).toBeNull();
  });
});
