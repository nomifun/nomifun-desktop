/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { parsePluginProjectId } from '@/common/types/ids';
import type {
  ApplyPluginSourceEditRequest,
  PluginProjectDetail,
} from '@/common/types/pluginPlatform';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import PluginSourceEditDialog from './PluginSourceEditDialog';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        pluginWorkbench: en,
      },
    },
  },
  interpolation: { escapeValue: false },
});

const projectId = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000051');
const sourceDigest = 'a'.repeat(64);

const detail: PluginProjectDetail = {
  summary: {
    project_id: projectId,
    project_revision: 4,
    display_name: 'Editable Plugin',
    source_state: 'editable',
    build_generation: 2,
    apply_mode: 'ask_before_apply',
    auto_apply_authorization_revision: 0,
    updated_at_ms: 1,
  },
  source_snapshot_digest: sourceDigest,
  dependency_lock_digest: 'b'.repeat(64),
  direct_dependencies: {},
};

const renderDialog = (
  props: Partial<React.ComponentProps<typeof PluginSourceEditDialog>> = {}
) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <PluginSourceEditDialog
        visible
        detail={detail}
        loading={false}
        onCancel={() => {}}
        onSubmit={() => {}}
        {...props}
      />
    </I18nextProvider>
  );

afterEach(() => cleanup());

describe('Plugin source edit dialog', () => {
  test('submits a normalized replace edit against the visible source snapshot', async () => {
    let request: ApplyPluginSourceEditRequest | undefined;
    renderDialog({
      onSubmit: (next) => {
        request = next;
      },
    });

    const dialog = within(document.body);
    await waitFor(() =>
      expect(
        dialog.getByRole('dialog', { name: 'Edit Plugin Source' })
      ).toBeDefined()
    );

    fireEvent.change(dialog.getByLabelText('Relative source path'), {
      target: { value: 'ui\\index.html' },
    });
    fireEvent.change(dialog.getByLabelText('File content'), {
      target: { value: '<main>updated</main>' },
    });
    fireEvent.click(dialog.getByRole('button', { name: 'Apply Source Edit' }));

    await waitFor(() => expect(request).toBeDefined());
    expect(request).toEqual({
      project_id: projectId,
      expected_source_snapshot_digest: sourceDigest,
      edit: {
        kind: 'replace',
        path: 'ui/index.html',
        content: '<main>updated</main>',
      },
    });
  });

  test('supports delete without rendering or submitting content', async () => {
    let request: ApplyPluginSourceEditRequest | undefined;
    renderDialog({
      onSubmit: (next) => {
        request = next;
      },
    });

    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Edit Plugin Source' });
    fireEvent.click(
      within(dialog.getByRole('radiogroup')).getByLabelText('Delete file')
    );
    expect(dialog.queryByLabelText('Content')).toBeNull();

    fireEvent.change(dialog.getByLabelText('Relative source path'), {
      target: { value: 'ui/old.html' },
    });
    fireEvent.click(dialog.getByRole('button', { name: 'Apply Source Edit' }));

    await waitFor(() => expect(request).toBeDefined());
    expect(request?.edit).toEqual({ kind: 'delete', path: 'ui/old.html' });
  });

  test('rejects traversal paths and empty replacement content locally', async () => {
    let submitted = 0;
    renderDialog({
      onSubmit: () => {
        submitted += 1;
      },
    });

    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Edit Plugin Source' });
    const path = dialog.getByLabelText('Relative source path');
    const content = dialog.getByLabelText('File content');
    const submit = dialog.getByRole('button', { name: 'Apply Source Edit' });

    fireEvent.change(path, { target: { value: '../outside.mjs' } });
    fireEvent.change(content, { target: { value: 'export default 1;' } });
    fireEvent.click(submit);
    expect(dialog.getByText('Enter a safe relative source path.')).toBeDefined();
    expect(submitted).toBe(0);

    fireEvent.change(path, { target: { value: 'ui/index.html' } });
    fireEvent.change(content, { target: { value: '   ' } });
    fireEvent.click(submit);
    expect(dialog.getByText('Enter file content before replacing a file.')).toBeDefined();
    expect(submitted).toBe(0);
  });

  test('keeps backend failure visible and disables destructive dismissal while loading', async () => {
    const page = renderDialog({
      loading: true,
      failure: { kind: 'error', message: 'Source snapshot is stale' },
    });
    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Edit Plugin Source' });
    expect(dialog.getByText('Source snapshot is stale')).toBeDefined();
    expect(
      dialog.getByRole('button', { name: 'Apply Source Edit' }).className.includes('loading')
    ).toBe(true);
    let canceled = 0;
    page.rerender(
      <I18nextProvider i18n={testI18n}>
        <PluginSourceEditDialog
          visible
          detail={detail}
          loading
          failure={{ kind: 'error', message: 'Source snapshot is stale' }}
          onCancel={() => {
            canceled += 1;
          }}
          onSubmit={() => {}}
        />
      </I18nextProvider>
    );
    fireEvent.click(page.getByRole('button', { name: 'Cancel' }));
    expect(canceled).toBe(0);
  });
});
