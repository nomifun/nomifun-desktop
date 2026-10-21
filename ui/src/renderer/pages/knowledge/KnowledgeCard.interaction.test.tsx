/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, jest, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import knowledge from '@/renderer/services/i18n/locales/en-US/knowledge.json';
import { KnowledgeCard, type KnowledgeCardProps } from './KnowledgeCard';

const i18n = createInstance();
await i18n.init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { knowledge } } },
  interpolation: { escapeValue: false },
});

afterEach(() => {
  cleanup();
  jest.useRealTimers();
});

function base(overrides: Partial<IKnowledgeBase> = {}): IKnowledgeBase {
  return {
    knowledge_base_id: parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000992'),
    name: 'Product notes',
    description: 'Domain terms and release procedures.',
    root_path: '/knowledge/product-notes',
    managed: true,
    tree_access: 'editable',
    created_at: 1,
    updated_at: Date.now(),
    file_count: 2,
    total_size: 1024,
    root_exists: true,
    tags: [],
    kind: 'blank',
    ...overrides,
  };
}

function mount(props: Partial<KnowledgeCardProps> = {}) {
  const value = { base: base(), ...props };
  const view = render(<I18nextProvider i18n={i18n}><KnowledgeCard {...value} /></I18nextProvider>);
  return { view, value };
}

describe('knowledge card navigation and actions', () => {
  test('opens the selected knowledge base from the card without a duplicate open action', () => {
    const onOpen = mock((_base: IKnowledgeBase) => {});
    const { view, value } = mount({ onOpen });
    const open = view.getByRole('button', { name: `Open: ${value.base.name}` });

    fireEvent.click(open);

    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith(value.base);
    expect(view.getAllByRole('button', { name: /^Open/ })).toHaveLength(1);
  });

  test.each(['Enter', ' '])('opens through the %s keyboard activation', key => {
    const onOpen = mock((_base: IKnowledgeBase) => {});
    const { view, value } = mount({ onOpen });
    const open = view.getByRole('button', { name: `Open: ${value.base.name}` });

    fireEvent.keyDown(open, { key });

    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith(value.base);
  });

  test.each(['Edit', 'Delete'])('the %s menu action targets this base without navigating', async action => {
    const onOpen = mock(() => {});
    const onEdit = mock(() => {});
    const onDelete = mock((_base: IKnowledgeBase) => {});
    const { view, value } = mount({ onOpen, onEdit, onDelete });

    fireEvent.click(view.getByRole('button', { name: /More/ }));
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.click(await view.findByRole('menuitem', { name: action }));

    expect(onOpen).not.toHaveBeenCalled();
    if (action === 'Edit') {
      expect(onEdit).toHaveBeenCalledTimes(1);
      expect(onEdit).toHaveBeenCalledWith(value.base);
      expect(onDelete).not.toHaveBeenCalled();
    } else {
      expect(onDelete).toHaveBeenCalledTimes(1);
      expect(onDelete.mock.calls[0]?.[0]).toEqual(value.base);
      expect(onEdit).not.toHaveBeenCalled();
    }
  });

  test('an opening card exposes busy feedback and rejects repeated navigation', () => {
    const onOpen = mock(() => {});
    const { view, value } = mount({ onOpen, opening: true });
    const open = view.getByRole('button', { name: `Open: ${value.base.name}` });

    fireEvent.click(open);
    fireEvent.keyDown(open, { key: 'Enter', code: 'Enter' });
    fireEvent.keyDown(open, { key: ' ', code: 'Space' });

    expect(onOpen).not.toHaveBeenCalled();
    expect(view.container.querySelector('[aria-busy="true"]')).not.toBeNull();
  });

  test('reserves an empty description area without showing placeholder copy', () => {
    const { view } = mount({ base: base({ description: '' }) });
    expect(view.queryByText(knowledge.card.noDescription)).toBeNull();
    expect(view.getByText('2 file(s)')).toBeTruthy();
    expect(view.getByText('1.0 KB')).toBeTruthy();
  });
});

describe('knowledge card prefetch', () => {
  test('waits for intentional hover and cancels when the pointer leaves or the card unmounts', () => {
    jest.useFakeTimers();
    const onPrefetch = mock(() => {});
    const { view, value } = mount({ onPrefetch });
    const open = view.getByRole('button', { name: `Open: ${value.base.name}` });

    fireEvent.mouseEnter(open);
    act(() => { jest.advanceTimersByTime(119); });
    expect(onPrefetch).not.toHaveBeenCalled();
    fireEvent.mouseLeave(open);
    act(() => { jest.advanceTimersByTime(1000); });
    expect(onPrefetch).not.toHaveBeenCalled();

    fireEvent.mouseEnter(open);
    act(() => { jest.advanceTimersByTime(120); });
    expect(onPrefetch).toHaveBeenCalledTimes(1);
    expect(onPrefetch).toHaveBeenCalledWith(value.base);

    fireEvent.mouseLeave(open);
    fireEvent.mouseEnter(open);
    view.unmount();
    act(() => { jest.advanceTimersByTime(1000); });
    expect(onPrefetch).toHaveBeenCalledTimes(1);
  });

  test.each(['focus', 'pointer'])('prefetches immediately for %s intent', intent => {
    const onPrefetch = mock(() => {});
    const { view, value } = mount({ onPrefetch });
    const open = view.getByRole('button', { name: `Open: ${value.base.name}` });
    if (intent === 'focus') fireEvent.focus(open);
    else fireEvent.pointerDown(open);
    expect(onPrefetch).toHaveBeenCalledWith(value.base);
  });
});

describe('knowledge card source and tags', () => {
  test('keeps source mode and unavailable-directory feedback visible', () => {
    const { view } = mount({ base: base({
      kind: 'web', root_exists: false,
      source: { kind: 'url', mode: 'live', entries: [{ url: 'https://example.com/docs' }] },
    }) });

    expect(view.container.textContent).toContain(knowledge.card.kindWeb);
    expect(view.container.textContent).toContain(knowledge.card.modeLive);
    expect(view.getByText(knowledge.card.rootMissing)).toBeTruthy();
  });

  test('keeps two visible tags and reveals every remaining label in the overflow tooltip', async () => {
    const tagMap: Record<string, IKnowledgeTag> = Object.fromEntries(
      ['Release', 'Product', 'Reference', 'Runbook'].map((label, index) => [
        `tag-${index}`, { key: `tag-${index}`, label, sortOrder: index },
      ])
    );
    const { view, value } = mount({ base: base({ tags: Object.keys(tagMap) }), tagMap });

    expect(view.getByText('Release')).toBeTruthy();
    expect(view.getByText('Product')).toBeTruthy();
    expect(view.queryByText('Reference')).toBeNull();
    expect(view.queryByText('Runbook')).toBeNull();
    expect(view.getByText('+2')).toBeTruthy();
    fireEvent.mouseEnter(view.getByRole('button', { name: `Open: ${value.base.name}` }));
    const tooltip = await view.findByRole('tooltip');
    for (const tag of Object.values(tagMap)) expect(tooltip.textContent).toContain(tag.label);
  });
});
