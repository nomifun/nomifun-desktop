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
import { parseMiniAppId } from '@/common/types/ids';
import type { MiniAppWorkshop } from '@/common/types/miniAppPlatform';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import MiniAppSourceEditDialog from './MiniAppSourceEditDialog';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: { miniApps: en } } },
  interpolation: { escapeValue: false },
});

const miniappId = parseMiniAppId('0190f5fe-7c00-7a00-8000-0000000000b1');
const sourceDigest = 'a'.repeat(64);
const workshop: MiniAppWorkshop = {
  miniapp: {
    miniapp_id: miniappId,
    product_revision: 4,
    display_name: 'Editable MiniApp',
    kind: 'ui_only',
    lifecycle: 'disabled',
    releases: { pointer_revision: 1, active_release_epoch: 0 },
    service_health: { state: 'not_applicable' },
    surface_available: false,
    updated_at_ms: 1,
  },
  project_id: '0190f5fe-7c00-7a00-8000-0000000000b2',
  project_revision: 5,
  publish_mode: 'manual',
  source_state: 'editable',
  build_generation: 3,
  source_snapshot_digest: sourceDigest,
  dependency_lock_digest: 'b'.repeat(64),
  config_schema: { schema_digest: 'c'.repeat(64), schema: {} },
  config: {
    config_revision: 1,
    schema_digest: 'c'.repeat(64),
    values: {},
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 1,
  credential_slots: [],
  capabilities: [],
};

const realFetch = globalThis.fetch;

afterEach(() => {
  cleanup();
  globalThis.fetch = realFetch;
});

describe('MiniApp Source edit dialog', () => {
  test('loads the existing file and saves against its exact Project head', async () => {
    let requestBody: unknown;
    globalThis.fetch = (async (_input, init) => {
      const isSave = init?.method === 'POST';
      if (isSave) {
        requestBody = JSON.parse(String(init?.body));
      }
      return new Response(
        JSON.stringify({
          success: true,
          data: isSave
            ? { ...workshop, project_revision: 6, build_generation: 4 }
            : {
                miniapp_id: miniappId,
                project_id: workshop.project_id,
                path: 'ui/index.html',
                content: '<main>before</main>',
                source_snapshot_digest: sourceDigest,
                build_generation: 3,
              },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    let saved: MiniAppWorkshop | undefined;
    render(
      <I18nextProvider i18n={i18n}>
        <MiniAppSourceEditDialog
          visible
          workshop={workshop}
          onCancel={() => {}}
          onSaved={(next) => {
            saved = next;
          }}
        />
      </I18nextProvider>
    );

    const dialog = within(document.body);
    await dialog.findByRole('dialog', { name: 'Edit MiniApp Source' });
    await waitFor(() =>
      expect(
        (dialog.getByLabelText('File content') as HTMLTextAreaElement).value
      ).toBe('<main>before</main>')
    );
    fireEvent.change(dialog.getByLabelText('File content'), {
      target: { value: '<main>after</main>' },
    });
    fireEvent.click(dialog.getByRole('button', { name: 'Save Source' }));

    await waitFor(() => expect(saved?.build_generation).toBe(4));
    expect(requestBody).toEqual({
      miniapp_id: miniappId,
      expected_product_revision: 4,
      project_id: workshop.project_id,
      expected_project_revision: 5,
      expected_build_generation: 3,
      expected_source_snapshot_digest: sourceDigest,
      path: 'ui/index.html',
      content: '<main>after</main>',
    });
  });
});
