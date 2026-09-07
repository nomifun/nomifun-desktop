/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import {
  cleanup,
  fireEvent,
  render,
  waitFor,
  within,
} from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import {
  parsePluginArtifactId,
  parsePluginMountId,
} from '@/common/types/ids';
import type {
  ConfigurePluginRequest,
  PluginDetail,
} from '@/common/types/pluginPlatform';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import PluginConfigurationDialog from './PluginConfigurationDialog';

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

const mountId = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000031');
const artifactId = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000032');
const hiddenSecret = 'must-never-appear-in-the-rendered-dialog';

const pluginDetail = (
  schema: unknown = {
    type: 'object',
    additionalProperties: false,
    required: ['endpoint'],
    properties: {
      endpoint: {
        type: 'string',
        title: 'Endpoint',
      },
      retries: {
        type: 'integer',
        title: 'Retries',
        minimum: 0,
        maximum: 9,
      },
      enabled: {
        type: 'boolean',
        title: 'Enabled',
      },
      mode: {
        type: 'string',
        title: 'Mode',
        enum: ['fast', 'careful'],
      },
    },
  }
): PluginDetail => ({
  summary: {
    mount_id: mountId,
    mount_revision: 13,
    display_name: 'Configurable Plugin',
    lifecycle: 'enabled',
    current: {
      package_id: 'dev.nomifun.configurable',
      package_version: '1.0.0',
      artifact_id: artifactId,
      artifact_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    },
    contribution_count: 1,
    updated_at_ms: 1,
  },
  capabilities: [],
  config_schema: {
    schema_digest: 'c'.repeat(64),
    schema,
  },
  config: {
    config_revision: 4,
    schema_digest: 'c'.repeat(64),
    values: {
      endpoint: 'https://old.example.test',
      retries: 2,
      enabled: true,
      mode: 'fast',
    },
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 6,
  credential_slots: [
    {
      slot_key: 'api_key',
      display_name: 'Provider API key',
      required: true,
      status: 'bound',
      credential_id: 'credential://existing',
    },
    {
      slot_key: 'telemetry',
      display_name: 'Telemetry token',
      required: false,
      status: 'unbound',
    },
  ],
  retained_data: false,
});

const renderDialog = (
  detail: PluginDetail,
  props: Partial<React.ComponentProps<typeof PluginConfigurationDialog>> = {}
) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <PluginConfigurationDialog
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

describe('Plugin configuration dialog', () => {
  test('renders typed controls and submits exact CAS', async () => {
    let request: ConfigurePluginRequest | undefined;
    renderDialog(pluginDetail(), {
      onSubmit: (next) => {
        request = next;
      },
    });
    const dialog = within(document.body);

    await waitFor(() =>
      expect(
        dialog.getByRole('dialog', { name: 'Config and credentials' })
      ).toBeDefined()
    );
    expect(dialog.getByLabelText('Endpoint') instanceof HTMLInputElement).toBe(
      true
    );
    expect(dialog.getByLabelText('Retries') instanceof HTMLInputElement).toBe(
      true
    );
    expect(dialog.getByLabelText('Enabled')).toBeDefined();
    expect(dialog.getByLabelText('Mode')).toBeDefined();
    expect(dialog.getByText('credential://existing')).toBeDefined();

    fireEvent.click(
      dialog.getByRole('button', { name: 'Save configuration' })
    );

    await waitFor(() => expect(request).toBeDefined());
    expect(request).toMatchObject({
      mount_id: mountId,
      expected_mount_revision: 13,
      expected_current_target_digest: 'a'.repeat(64),
      expected_config_revision: 4,
      expected_schema_digest: 'c'.repeat(64),
      expected_credential_bindings_revision: 6,
      credential_bindings: {
        api_key: 'credential://existing',
        telemetry: null,
      },
    });
    expect(request?.values.endpoint).toBe('https://old.example.test');
  });

  test('keeps the selected edit mode mounted when a backend CAS error is reported', async () => {
    const detail = pluginDetail();
    const rendered = renderDialog(detail);
    const dialog = within(document.body);
    await dialog.findByText('Provider API key');
    const actions = dialog.getByLabelText(
      'Choose the Credential binding action for Provider API key'
    );
    fireEvent.click(within(actions).getByText('Use Credential ID'));
    expect(
      dialog.getByLabelText('Credential ID for Provider API key')
    ).toBeDefined();

    rendered.rerender(
      <I18nextProvider i18n={testI18n}>
        <PluginConfigurationDialog
          visible
          detail={detail}
          loading={false}
          failure={{ message: 'Exact CAS is stale' }}
          onCancel={() => {}}
          onSubmit={() => {}}
        />
      </I18nextProvider>
    );

    expect(dialog.getByText('Exact CAS is stale')).toBeDefined();
    expect(
      dialog.getByLabelText('Credential ID for Provider API key')
    ).toBeDefined();
  });

  test('offers controlled Credential ID replacement and optional unbind actions', async () => {
    renderDialog(pluginDetail());
    const dialog = within(document.body);
    await dialog.findByText('Provider API key');

    const apiKeyActions = dialog.getByLabelText(
      'Choose the Credential binding action for Provider API key'
    );
    fireEvent.click(within(apiKeyActions).getByText('Use Credential ID'));
    const credentialInput = dialog.getByLabelText(
      'Credential ID for Provider API key'
    ) as HTMLInputElement;
    expect(credentialInput.value).toBe('');

    const telemetryActions = dialog.getByLabelText(
      'Choose the Credential binding action for Telemetry token'
    );
    expect(within(telemetryActions).getByText('Unbind')).toBeDefined();
  });

  test('makes unsupported schemas visibly read-only without a raw JSON editor', async () => {
    const detail = pluginDetail({
      type: 'object',
      additionalProperties: false,
      properties: {
        nested: {
          type: 'object',
          properties: {
            value: { type: 'string' },
          },
        },
      },
    });
    detail.config.values = {};
    renderDialog(detail);
    const dialog = within(document.body);

    await dialog.findByText(
      'This config schema is read-only in the current editor'
    );
    expect(
      dialog
        .getByRole('button', { name: 'Save configuration' })
        .hasAttribute('disabled')
    ).toBe(true);
    expect(dialog.queryByRole('textbox')).toBeNull();
  });

  test('rejects secret config fields without rendering or returning their values', async () => {
    const detail = pluginDetail({
      type: 'object',
      additionalProperties: false,
      properties: {
        endpoint: {
          type: 'string',
          title: 'Endpoint',
        },
        password: {
          type: 'string',
          title: 'Password',
        },
      },
    });
    detail.config.values = {
      endpoint: 'https://safe.example.test',
      password: hiddenSecret,
    };
    let submissions = 0;
    renderDialog(detail, {
      onSubmit: () => {
        submissions += 1;
      },
    });
    const dialog = within(document.body);

    await dialog.findByText(
      /Plugin config cannot store secrets; the author must declare a Credential slot instead/
    );
    expect(document.body.textContent?.includes(hiddenSecret)).toBe(false);
    expect(dialog.queryByLabelText('Password')).toBeNull();
    expect(
      (dialog.getByLabelText('Endpoint') as HTMLInputElement).hasAttribute(
        'disabled'
      )
    ).toBe(true);
    const save = dialog.getByRole('button', { name: 'Save configuration' });
    expect(save.hasAttribute('disabled')).toBe(true);
    fireEvent.click(save);
    expect(submissions).toBe(0);
  });
});
