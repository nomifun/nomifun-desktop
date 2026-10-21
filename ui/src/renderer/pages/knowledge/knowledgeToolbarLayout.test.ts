import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { createElement, useState } from 'react';
import { I18nextProvider } from 'react-i18next';
import knowledge from '@/renderer/services/i18n/locales/en-US/knowledge.json';
import KnowledgeTagFilterBar, { type KnowledgeTagFilterBarProps } from './KnowledgeTagFilterBar';

const i18n = createInstance();
await i18n.init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { knowledge } } },
  interpolation: { escapeValue: false },
});
afterEach(cleanup);

function mount() {
  const kindChanged = mock((_kind: KnowledgeTagFilterBarProps['kindFilter']) => {});
  const tagsChanged = mock((_tags: string[]) => {});
  const sortChanged = mock((_sort: KnowledgeTagFilterBarProps['sort']) => {});
  const directionChanged = mock((_direction: KnowledgeTagFilterBarProps['sortDirection']) => {});

  function Harness() {
    const [kindFilter, setKindFilter] = useState<KnowledgeTagFilterBarProps['kindFilter']>(null);
    const [tagFilter, setTagFilter] = useState<string[]>([]);
    const [sort, setSort] = useState<KnowledgeTagFilterBarProps['sort']>('updated');
    const [sortDirection, setSortDirection] = useState<KnowledgeTagFilterBarProps['sortDirection']>('desc');
    return createElement(KnowledgeTagFilterBar, {
      kindFilter, tagFilter, sort, sortDirection,
      kindCounts: { blank: 1, local: 2, web: 3 },
      tagCounts: { release: 2, product: 3 },
      tags: [
        { key: 'release', label: 'Release', sortOrder: 0 },
        { key: 'product', label: 'Product', sortOrder: 1 },
      ],
      onKindChange: kind => { kindChanged(kind); setKindFilter(kind); },
      onTagChange: tags => { tagsChanged(tags); setTagFilter(tags); },
      onSortChange: next => { sortChanged(next); setSort(next); },
      onSortDirectionChange: next => { directionChanged(next); setSortDirection(next); },
    });
  }

  const view = render(createElement(I18nextProvider, { i18n }, createElement(Harness)));
  return { view, kindChanged, tagsChanged, sortChanged, directionChanged };
}

describe('Knowledge toolbar controls', () => {
  test('retains each knowledge type and can return to all types', async () => {
    const { view, kindChanged } = mount();
    for (const [label, kind] of [['Blank', 'blank'], ['Local', 'local'], ['Web', 'web'], ['All', null]] as const) {
      fireEvent.click(view.getByRole('button', { name: /^Type/ }));
      fireEvent.click(await view.findByRole('menuitem', { name: new RegExp(`^${label}\\b`) }));
      expect(kindChanged.mock.calls.at(-1)?.[0]).toBe(kind);
    }
  });

  test('preserves multiple tag selections and lets users remove or clear them', async () => {
    const { view, tagsChanged } = mount();
    fireEvent.click(view.getByRole('button', { name: /^Tags/ }));
    fireEvent.click(await view.findByRole('menuitem', { name: /^Release/ }));
    expect(tagsChanged.mock.calls.at(-1)?.[0]).toEqual(['release']);

    fireEvent.click(view.getByRole('button', { name: /^Tags/ }));
    fireEvent.click(await view.findByRole('menuitem', { name: /^Product/ }));
    expect(tagsChanged.mock.calls.at(-1)?.[0]).toEqual(['release', 'product']);

    fireEvent.click(view.getByRole('button', { name: /^Release/ }));
    expect(tagsChanged.mock.calls.at(-1)?.[0]).toEqual(['product']);

    fireEvent.click(view.getByRole('button', { name: /^Tags/ }));
    fireEvent.click(await view.findByRole('menuitem', { name: /^All/ }));
    expect(tagsChanged.mock.calls.at(-1)?.[0]).toEqual([]);
  });

  test('keeps all sort fields and both directions independent of the filters', async () => {
    const { view, kindChanged, tagsChanged, sortChanged, directionChanged } = mount();
    for (const [label, sort] of [['Created', 'created'], ['Name', 'name'], ['Size', 'size'], ['Recently updated', 'updated']] as const) {
      fireEvent.click(view.getByRole('button', { name: /^Sort/ }));
      fireEvent.click(await view.findByRole('menuitem', { name: label }));
      expect(sortChanged.mock.calls.at(-1)?.[0]).toBe(sort);
    }

    fireEvent.click(view.getByRole('button', { name: 'Current: Descending. Click to switch to Ascending' }));
    expect(directionChanged.mock.calls.at(-1)?.[0]).toBe('asc');
    fireEvent.click(view.getByRole('button', { name: 'Current: Ascending. Click to switch to Descending' }));
    expect(directionChanged.mock.calls.at(-1)?.[0]).toBe('desc');
    expect(kindChanged).not.toHaveBeenCalled();
    expect(tagsChanged).not.toHaveBeenCalled();
  });
});
