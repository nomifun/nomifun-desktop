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
  PluginProjectDetail,
  UpdatePluginDependenciesRequest,
} from '@/common/types/pluginPlatform';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import PluginDependencyDialog from './PluginDependencyDialog';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: { pluginWorkbench: en } } },
  interpolation: { escapeValue: false },
});

const projectId = parsePluginProjectId('0190f5fe-7c00-7a00-8000-000000000061');
const detail: PluginProjectDetail = {
  summary: {
    project_id: projectId,
    project_revision: 8,
    display_name: 'Dependency Plugin',
    source_state: 'editable',
    build_generation: 3,
    apply_mode: 'ask_before_apply',
    auto_apply_authorization_revision: 0,
    updated_at_ms: 1,
  },
  source_snapshot_digest: 'a'.repeat(64),
  dependency_lock_digest: 'b'.repeat(64),
  direct_dependencies: { existing: '1.0.0' },
};

const renderDialog = (
  onSubmit: (request: UpdatePluginDependenciesRequest) => void
) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <PluginDependencyDialog
        visible
        detail={detail}
        loading={false}
        onCancel={() => {}}
        onSubmit={onSubmit}
      />
    </I18nextProvider>
  );

afterEach(() => cleanup());

describe('Plugin dependency dialog', () => {
  test('preloads current dependencies and submits the complete exact-CAS map', async () => {
    let request: UpdatePluginDependenciesRequest | undefined;
    renderDialog((next) => {
      request = next;
    });
    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Update Plugin dependencies' });
    const input = dialog.getByLabelText('Direct dependencies (JSON)') as HTMLTextAreaElement;
    expect(input.value).toContain('"existing": "1.0.0"');
    fireEvent.input(input, {
      target: { value: '{"alpha":"^1.0.0"}' },
    });
    await waitFor(() => expect(input.value).toBe('{"alpha":"^1.0.0"}'));
    fireEvent.click(dialog.getByRole('button', { name: 'Resolve and update' }));

    await waitFor(() => expect(request).toBeDefined());
    expect(request).toEqual({
      project_id: projectId,
      expected_project_revision: 8,
      expected_build_generation: 3,
      expected_source_snapshot_digest: 'a'.repeat(64),
      expected_dependency_lock_digest: 'b'.repeat(64),
      dependencies: { alpha: '^1.0.0' },
    });
  });

  test('rejects non-string dependency requirements locally', async () => {
    let submitted = false;
    renderDialog(() => {
      submitted = true;
    });
    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Update Plugin dependencies' });
    const input = dialog.getByLabelText('Direct dependencies (JSON)') as HTMLTextAreaElement;
    fireEvent.input(input, { target: { value: '{"alpha":1}' } });
    await waitFor(() => expect(input.value).toBe('{"alpha":1}'));
    fireEvent.click(dialog.getByRole('button', { name: 'Resolve and update' }));

    await waitFor(() =>
      expect(
        dialog.getByText(
          'Dependencies must be a JSON object whose values are SemVer strings.'
        )
      ).toBeDefined()
    );
    expect(submitted).toBe(false);
  });
});
