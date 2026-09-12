import '../../../../test/setup-dom.ts';
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
  cleanup,
  fireEvent,
  render,
  waitFor,
  within,
} from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import {
  miniAppProduct,
  type MiniAppWorkspace,
} from '@/common/adapter/miniAppProductBridge';
import { parseMiniAppId } from '@/common/types/ids';
import type { MiniAppSummary } from '@/common/types/miniAppPlatform';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import MiniAppLibraryPage from './MiniAppLibraryPage';
import { selectLibraryApps } from './libraryState';

const i18n = createInstance();
await i18n
  .use(initReactI18next)
  .init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { miniApps: en } } },
    interpolation: { escapeValue: false },
  });
const group = '0190f5fe-7c00-7000-8000-000000000099';
function app(index: number): MiniAppSummary {
  return {
    miniapp_id: parseMiniAppId(
      `0190f5fe-7c00-7000-8000-${String(index + 1).padStart(12, '0')}`,
    ),
    display_name: `Tool ${String(index).padStart(3, '0')}`,
    description: `Purpose ${index}`,
    kind: 'ui_only',
    lifecycle: 'enabled',
    product_revision: 1,
    updated_at_ms: 1000 + index,
    surface_available: true,
    service_health: { state: 'not_applicable' },
    releases: {
      pointer_revision: 1,
      active_release_epoch: 1,
      active: {
        release_id: 'release',
        artifact_id: 'artifact',
        release_digest: 'digest',
        manifest_digest: 'manifest',
      },
    },
  };
}
const apps = Array.from({ length: 200 }, (_, i) => app(i));
let workspace: MiniAppWorkspace;
let mocks: Array<{ mockRestore: () => void }> = [];
function replaceInvoke<T extends { invoke: unknown }>(
  target: T,
  implementation: T['invoke'],
) {
  const original = target.invoke;
  target.invoke = implementation;
  return {
    mockRestore() {
      target.invoke = original;
    },
  };
}
beforeEach(() => {
  workspace = {
    revision: 0,
    collections: [{ id: group, name: 'Work' }],
    items: {},
  };
  mocks = [
    replaceInvoke(ipcBridge.miniapps.library, async () => ({
      library_revision: 1,
      miniapps: apps,
    })),
    replaceInvoke(miniAppProduct.workspace, async () =>
      structuredClone(workspace),
    ),
    replaceInvoke(miniAppProduct.drafts, async () => []),
    replaceInvoke(
      miniAppProduct.updateWorkspace,
      async (value: MiniAppWorkspace) => {
        workspace = { ...structuredClone(value), revision: value.revision + 1 };
        return structuredClone(workspace);
      },
    ),
  ];
});
afterEach(() => {
  cleanup();
  mocks.forEach((mock) => mock.mockRestore());
});
function renderLibrary() {
  return render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <MiniAppLibraryPage />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

describe('MiniApp collection product', () => {
  test('200 applications stay paged and can be found by purpose', async () => {
    const ui = renderLibrary();
    await waitFor(() =>
      expect(ui.getAllByRole('button', { name: /^Open Tool/ }).length).toBe(12),
    );
    fireEvent.input(ui.getByRole('textbox', { name: en.product.search }), {
      target: { value: 'Purpose 143' },
    });
    await waitFor(() =>
      expect(ui.getAllByRole('button', { name: /^Open Tool/ }).length).toBe(1),
    );
    expect(ui.getByRole('button', { name: 'Open Tool 143' })).toBeTruthy();
  });
  test('moving selected apps persists collection membership without changing pins', async () => {
    workspace.items[apps[199].miniapp_id] = {
      collection_id: null,
      pinned: true,
      last_opened: 0,
      name: null,
    };
    const ui = renderLibrary();
    await waitFor(() =>
      expect(ui.getByRole('textbox', { name: en.product.search })).toBeTruthy(),
    );
    fireEvent.input(ui.getByRole('textbox', { name: en.product.search }), {
      target: { value: '' },
    });
    await waitFor(() =>
      expect(ui.getByRole('button', { name: 'Open Tool 199' })).toBeTruthy(),
    );
    fireEvent.click(ui.getByRole('button', { name: en.product.organize }));
    fireEvent.click(ui.getByRole('checkbox', { name: 'Select Tool 199' }));
    fireEvent.click(ui.getByRole('button', { name: en.product.move }));
    const dialog = ui.getByRole('dialog');
    fireEvent.change(within(dialog).getByRole('combobox'), {
      target: { value: group },
    });
    fireEvent.click(
      within(dialog).getByRole('button', { name: en.product.confirm }),
    );
    await waitFor(() =>
      expect(workspace.items[apps[199].miniapp_id].collection_id).toBe(group),
    );
    expect(workspace.items[apps[199].miniapp_id].pinned).toBe(true);
  });
  test('filtering keeps drafts and deleted items out of the usable collection', () => {
    const pending = app(201);
    pending.releases.active = undefined;
    const trashed = app(202);
    trashed.lifecycle = 'trashed';
    expect(
      selectLibraryApps(
        [...apps, pending, trashed],
        workspace,
        'all',
        '',
        'name',
      ).length,
    ).toBe(200);
    expect(
      selectLibraryApps(
        [...apps, pending, trashed],
        workspace,
        'trash',
        '',
        'name',
      ),
    ).toEqual([trashed]);
    workspace.items[apps[0].miniapp_id] = {
      collection_id: group,
      pinned: true,
      last_opened: 100,
      name: 'Daily checklist',
    };
    expect(
      selectLibraryApps(apps, workspace, group, 'Daily', 'recent'),
    ).toEqual([apps[0]]);
    expect(
      selectLibraryApps(apps, workspace, 'unfiled', 'Daily', 'recent'),
    ).toEqual([]);
  });
});
