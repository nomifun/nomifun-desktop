/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import assert from 'node:assert/strict';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';

import type { CreativeModelCatalogSnapshot } from '../../models';
import type { TemplateDraftPort } from '../agent';
import TemplateAgentDraftModal from './TemplateAgentDraftModal';

const SUCCESS_MARKER = 'template-model-select-interaction:ok';
const PROVIDER_ID = parseProviderId('0190f5fe-7c00-7a00-8000-000000000961');
const provider: IProvider = {
  id: PROVIDER_ID,
  platform: 'openai',
  name: 'QA Template Mock',
  base_url: 'http://127.0.0.1:18789/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  enabled: true,
  models: [
    {
      provider_id: PROVIDER_ID,
      model: 'qa-template-chat',
      enabled: true,
      sort_order: 0,
      capabilities: [
        {
          task: 'chat',
          traits: [],
          protocol: 'openai.chat_text',
          connection_role: 'default',
          allow_cross_origin_credentials: false,
          provider_params: {},
          created_at: 1,
          updated_at: 1,
        },
      ],
      created_at: 1,
      updated_at: 1,
    },
  ],
};
const catalog: CreativeModelCatalogSnapshot = {
  status: 'ready',
  providers: [provider],
  error: null,
};

let backendCalls = 0;
const unusedPort: TemplateDraftPort = {
  async draft() {
    backendCalls += 1;
    throw new Error('This selection test must not invoke the backend.');
  },
};

const LEGACY_REF_WARNING =
  'Accessing element.ref was removed in React 19. ref is now a regular prop. It will be removed from the JSX Element type in a future release.';
const HAPPY_DOM_HEIGHT_WARNING =
  '`NaN` is an invalid value for the `%s` css style property.';

const knownHarnessWarning = (args: unknown[]): boolean =>
  (args.length === 1 && args[0] === LEGACY_REF_WARNING) ||
  (args.length === 2 && args[0] === HAPPY_DOM_HEIGHT_WARNING && args[1] === 'height');

const run = async (): Promise<void> => {
  const testI18n = createInstance();
  await testI18n.use(initReactI18next).init({
    lng: 'zh-CN',
    fallbackLng: 'zh-CN',
    resources: { 'zh-CN': { translation: {} } },
    interpolation: { escapeValue: false },
  });

  const unexpectedConsoleErrors: unknown[][] = [];
  const originalConsoleError = console.error;
  console.error = (...args: unknown[]) => {
    // Arco 2 reads the legacy React element.ref field, and its textarea
    // autosizer cannot measure layout in Happy DOM. Keep these two exact
    // harness-only warnings local while surfacing every other console error.
    if (knownHarnessWarning(args)) return;
    unexpectedConsoleErrors.push(args);
    originalConsoleError(...args);
  };

  try {
    const portalRoot = document.createElement('div');
    portalRoot.id = 'creative-studio-portal-root';
    document.body.append(portalRoot);

    render(
      <I18nextProvider i18n={testI18n}>
        <TemplateAgentDraftModal
          visible
          catalog={catalog}
          port={unusedPort}
          onApply={() => undefined}
          onClose={() => undefined}
        />
      </I18nextProvider>
    );

    const dialog = await screen.findByRole('dialog', { name: 'Create template with AI' });
    const modal = within(dialog);
    fireEvent.change(modal.getByRole('textbox', { name: 'Template request' }), {
      target: { value: 'Create a product hero template' },
    });
    fireEvent.click(modal.getByRole('combobox', { name: 'Chat model' }));

    const option = await screen.findByRole('option', {
      name: 'qa-template-chat openai.chat_text',
    });
    assert.equal(dialog.contains(option), true, 'the popup must remain inside the FocusLock');
    fireEvent.click(option);

    await waitFor(() => {
      const selector = dialog.querySelector('[data-creative-model-select]');
      assert.equal(selector?.getAttribute('data-selection-state'), 'resolved');
      assert.match(selector?.textContent ?? '', /qa-template-chat/);
      assert.equal(
        (modal.getByRole('button', { name: 'Generate template draft' }) as HTMLButtonElement)
          .disabled,
        false
      );
    });

    assert.equal(backendCalls, 0, 'selection must not invoke the draft backend');
  } finally {
    try {
      cleanup();
      document.body.replaceChildren();
      await new Promise<void>((resolve) => setImmediate(resolve));
      await (
        globalThis as typeof globalThis & {
          happyDOM?: { waitUntilComplete(): Promise<void> };
        }
      ).happyDOM?.waitUntilComplete();
      await new Promise<void>((resolve) => setImmediate(resolve));
    } finally {
      console.error = originalConsoleError;
    }
  }

  assert.equal(
    unexpectedConsoleErrors.length,
    0,
    'the interaction emitted an unexpected console error'
  );
};

try {
  await run();
  console.log(SUCCESS_MARKER);
  process.exitCode = 0;
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
