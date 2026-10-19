/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { CreativeCanvasSummary } from '../domain';
import CreativeStudioCanvasesPage, { type CreativeStudioCanvasesPageProps } from './CreativeStudioCanvasesPage';
import { CREATIVE_STUDIO_CANVAS_FIXTURES, createCreativeStudioCanvasFixture } from './testing';
import type { CreativeStudioCanvasesService } from './types';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

afterEach(cleanup);

function mount(options: {
  archiveAvailable?: boolean;
  props?: CreativeStudioCanvasesPageProps;
} = {}) {
  const canvases = CREATIVE_STUDIO_CANVAS_FIXTURES.slice(0, 2);
  const calls = {
    opened: [] as string[],
    prefetched: [] as string[],
    created: [] as string[],
    imported: [] as File[],
    renamed: [] as Array<{ canvasId: string; title: string }>,
    exported: [] as string[][],
    deleted: [] as string[][],
  };
  const created = createCreativeStudioCanvasFixture({ canvasId: 'canvas-created' });
  const imported = createCreativeStudioCanvasFixture({ canvasId: 'canvas-imported', title: 'Imported canvas' });
  const service: CreativeStudioCanvasesService = {
    archiveCapabilities: {
      canImport: options.archiveAvailable ?? true,
      canExport: options.archiveAvailable ?? true,
    },
    listCanvases: async () => canvases,
    createCanvas: async (title) => {
      calls.created.push(title);
      return { ...created, title };
    },
    importCanvasArchive: async (file) => {
      calls.imported.push(file);
      return [imported];
    },
    renameCanvas: async (canvasId, title) => {
      calls.renamed.push({ canvasId, title });
      return { ...canvases.find((canvas) => canvas.canvasId === canvasId)!, title };
    },
    deleteCanvases: async (ids) => { calls.deleted.push([...ids]); },
    exportCanvases: async (ids) => { calls.exported.push([...ids]); },
  };
  const props: CreativeStudioCanvasesPageProps = {
    autoLoad: false,
    initialSnapshot: { status: 'ready', canvases },
    service,
    onOpenCanvas: (canvas) => calls.opened.push(canvas.canvasId),
    onPrefetchCanvas: (canvas) => calls.prefetched.push(canvas.canvasId),
    ...options.props,
  };
  const element = (nextProps = props) => (
    <I18nextProvider i18n={testI18n}>
      <CreativeStudioCanvasesPage {...nextProps} />
    </I18nextProvider>
  );
  const view = render(element());
  const body = within(view.container.ownerDocument.body);
  const card = (canvas: CreativeCanvasSummary) => {
    const node = view.container.querySelector<HTMLElement>(`[data-canvas-id="${canvas.canvasId}"]`);
    expect(node).not.toBeNull();
    return within(node!);
  };
  const openMenu = async (canvas = canvases[0]) => {
    fireEvent.click(card(canvas).getByRole('button', { name: `More actions: ${canvas.title}` }));
    return body.findByRole('menu');
  };
  return { ...view, body, card, openMenu, calls, canvases, created, imported, props, element };
}

describe('compact Canvas library interactions', () => {
  test('retains native open activation and prefetch without exposing batch controls by default', () => {
    const view = mount();
    const canvas = view.canvases[0];
    const open = view.card(canvas).getByRole('button', { name: `Open canvas: ${canvas.title}` }) as HTMLButtonElement;
    expect(open.tagName).toBe('BUTTON');
    expect(open.type).toBe('button');
    expect(view.queryByRole('checkbox')).toBeNull();
    fireEvent.focus(open);
    expect(view.calls.prefetched).toContain(canvas.canvasId);
    fireEvent.click(open);
    // Native buttons also receive detail=0 clicks from keyboard activation.
    fireEvent.click(open, { detail: 0 });
    expect(view.calls.opened).toEqual([canvas.canvasId, canvas.canvasId]);

    view.rerender(view.element({ ...view.props, openingCanvasId: canvas.canvasId }));
    const pending = view.card(canvas).getByRole('button', { name: `Open canvas: ${canvas.title}` }) as HTMLButtonElement;
    expect(pending.disabled).toBe(true);
    fireEvent.click(pending);
    expect(view.calls.opened).toHaveLength(2);
    expect(view.card(canvas).getByRole('status').textContent).toContain(canvas.title);
  });

  test('selects, clears, selects all and exports exact batch identities without opening a canvas', async () => {
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: 'Manage selection' }));
    const canvas = view.canvases[0];
    const cardSelect = view.card(canvas).getByRole('button', { name: `Select ${canvas.title}` });
    fireEvent.click(cardSelect, { detail: 0 });
    expect(view.getByText('1 selected')).toBeTruthy();
    fireEvent.click(cardSelect);
    expect(view.getByText('0 selected')).toBeTruthy();
    const selected = view.getByRole('checkbox', { name: `Select ${canvas.title}` }) as HTMLInputElement;
    fireEvent.click(selected);
    expect(selected.checked).toBe(true);
    expect(view.getByText('1 selected')).toBeTruthy();
    fireEvent.keyDown(selected, { key: ' ', code: 'Space' });
    expect(view.calls.opened).toEqual([]);
    fireEvent.click(selected);
    expect(selected.checked).toBe(false);
    expect(view.getByText('0 selected')).toBeTruthy();

    fireEvent.click(view.getByRole('checkbox', { name: 'Select all' }));
    expect(view.getByText('2 selected')).toBeTruthy();
    fireEvent.click(view.getByRole('checkbox', { name: 'Select all' }));
    expect(view.getByText('0 selected')).toBeTruthy();
    fireEvent.click(view.getByRole('checkbox', { name: 'Select all' }));
    fireEvent.click(view.getByRole('button', { name: 'Export selected' }));
    await waitFor(() => expect(view.calls.exported).toEqual([view.canvases.map((item) => item.canvasId)]));
    await waitFor(() => expect((view.getByRole('button', { name: 'Done' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(view.getByRole('button', { name: 'Done' }));
    expect(view.queryByRole('checkbox')).toBeNull();
    expect(view.calls.opened).toEqual([]);
  });

  test('exports from the card menu and renames names with spaces without opening the canvas', async () => {
    const view = mount();
    const canvas = view.canvases[0];
    const menu = within(await view.openMenu(canvas));
    fireEvent.click(menu.getByRole('menuitem', { name: 'Export' }));
    await waitFor(() => expect(view.calls.exported).toEqual([[canvas.canvasId]]));
    expect(view.calls.opened).toEqual([]);
    await waitFor(() => expect((view.card(canvas).getByRole('button', { name: `More actions: ${canvas.title}` }) as HTMLButtonElement).disabled).toBe(false));

    fireEvent.click(within(await view.openMenu(canvas)).getByRole('menuitem', { name: 'Rename' }));
    const input = view.getByRole('textbox', { name: 'Enter a canvas name' }) as HTMLInputElement;
    const space = new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true, cancelable: true });
    fireEvent(input, space);
    expect(space.defaultPrevented).toBe(false);
    fireEvent.change(input, { target: { value: '  Brand film concept  ' } });
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true });
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 });
    expect(view.calls.renamed).toEqual([]);
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.calls.renamed).toEqual([{ canvasId: canvas.canvasId, title: 'Brand film concept' }]));
    await view.findByRole('heading', { name: 'Brand film concept' });
    expect(view.calls.opened).toEqual([]);

    const renamed = { ...canvas, title: 'Brand film concept' };
    fireEvent.click(within(await view.openMenu(renamed)).getByRole('menuitem', { name: 'Rename' }));
    fireEvent.change(view.getByRole('textbox'), { target: { value: 'Discarded draft' } });
    fireEvent.keyDown(view.getByRole('textbox'), { key: 'Escape' });
    expect(view.queryByRole('textbox')).toBeNull();
    expect(view.calls.renamed).toHaveLength(1);
    expect(view.calls.opened).toEqual([]);
  });

  test('requires confirmation for individual and selected deletion and preserves cancellation', async () => {
    const view = mount({ props: { initialSelectedIds: [CREATIVE_STUDIO_CANVAS_FIXTURES[1].canvasId] } });
    const canvas = view.canvases[0];
    fireEvent.click(within(await view.openMenu(canvas)).getByRole('menuitem', { name: 'Delete' }));
    const firstDialog = await view.body.findByRole('dialog', { name: 'Delete canvases?' });
    expect(view.calls.deleted).toEqual([]);
    fireEvent.click(within(firstDialog).getByRole('button', { name: 'Cancel' }));
    expect(view.calls.deleted).toEqual([]);

    fireEvent.click(view.getByRole('button', { name: 'Delete selected' }));
    const selectedDialog = await view.body.findByRole('dialog', { name: 'Delete canvases?' });
    fireEvent.click(within(selectedDialog).getByRole('button', { name: 'Delete' }));
    const selectedId = view.canvases[1].canvasId;
    await waitFor(() => expect(view.calls.deleted).toEqual([[selectedId]]));
    await waitFor(() => expect(view.container.querySelector(`[data-canvas-id="${selectedId}"]`)).toBeNull());
    expect(view.container.querySelector(`[data-canvas-id="${canvas.canvasId}"]`)).not.toBeNull();
    expect(view.calls.opened).toEqual([]);

    fireEvent.click(within(await view.openMenu(canvas)).getByRole('menuitem', { name: 'Delete' }));
    fireEvent.click(within(await view.body.findByRole('dialog', { name: 'Delete canvases?' })).getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(view.calls.deleted).toEqual([[selectedId], [canvas.canvasId]]));
    await view.findByText('No canvases yet');
  });

  test('keeps unavailable archive actions disabled in both card and batch controls', async () => {
    const view = mount({ archiveAvailable: false, props: { initialSelectedIds: [CREATIVE_STUDIO_CANVAS_FIXTURES[0].canvasId] } });
    const importButton = view.getByRole('button', { name: 'Import canvas' }) as HTMLButtonElement;
    const exportBatch = view.getByRole('button', { name: 'Export selected' }) as HTMLButtonElement;
    expect(importButton.disabled).toBe(true);
    expect(exportBatch.disabled).toBe(true);
    fireEvent.click(importButton);
    fireEvent.click(exportBatch);
    const canvas = view.canvases[0];
    const trigger = view.card(canvas).getByRole('button', { name: `More actions: ${canvas.title}` });
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    const menu = within(await view.body.findByRole('menu'));
    const exportItem = menu.getByRole('menuitem', { name: 'Export' }) as HTMLButtonElement;
    expect(exportItem.disabled).toBe(true);
    fireEvent.click(exportItem);
    const rename = menu.getByRole('menuitem', { name: 'Rename' });
    const remove = menu.getByRole('menuitem', { name: 'Delete' });
    expect(document.activeElement).toBe(rename);
    fireEvent.keyDown(rename, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(remove);
    fireEvent.keyDown(remove, { key: 'Home' });
    expect(document.activeElement).toBe(rename);
    fireEvent.keyDown(rename, { key: 'End' });
    expect(document.activeElement).toBe(remove);
    fireEvent.keyDown(remove, { key: 'Escape' });
    expect(document.activeElement).toBe(trigger);
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(view.calls.exported).toEqual([]);
    expect(view.calls.imported).toEqual([]);
    expect(view.calls.opened).toEqual([]);
  });

  test('retains delete-all behind the page menu and confirmation', async () => {
    const view = mount();
    fireEvent.click(view.getByRole('button', { name: 'More actions' }));
    fireEvent.click(await view.body.findByRole('menuitem', { name: 'Delete all' }));
    const dialog = await view.body.findByRole('dialog', { name: 'Delete canvases?' });
    expect(within(dialog).getByText(/2 canvases and all of their nodes/)).toBeTruthy();
    expect(view.calls.deleted).toEqual([]);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(view.calls.deleted).toEqual([view.canvases.map((canvas) => canvas.canvasId)]));
    await view.findByText('No canvases yet');
  });

  test('retains archive input import and new-canvas creation', async () => {
    const view = mount();
    const fileInput = view.container.querySelector<HTMLInputElement>('input[type="file"]');
    expect(fileInput).not.toBeNull();
    let chooserOpened = false;
    fileInput!.addEventListener('click', () => { chooserOpened = true; });
    fireEvent.click(view.getByRole('button', { name: 'Import canvas' }));
    expect(chooserOpened).toBe(true);
    const file = new File(['canvas archive'], 'creative.nomifun-canvas.zip', { type: 'application/zip' });
    await act(async () => fireEvent.change(fileInput!, { target: { files: [file] } }));
    expect(view.calls.imported).toEqual([file]);
    await view.findByRole('heading', { name: 'Imported canvas' });
    expect(view.calls.opened).toEqual([]);

    await act(async () => fireEvent.click(view.getByRole('button', { name: 'New canvas' })));
    expect(view.calls.created).toEqual(['Infinite canvas 4']);
    expect(view.calls.opened).toEqual([view.created.canvasId]);
    expect(view.container.querySelector(`[data-canvas-id="${view.created.canvasId}"]`)).not.toBeNull();
  });
});
