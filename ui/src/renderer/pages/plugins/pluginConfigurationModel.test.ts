/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  parsePluginArtifactId,
  parsePluginMountId,
} from '@/common/types/ids';
import type { PluginDetail } from '@/common/types/pluginPlatform';
import {
  configurePluginRequest,
  createPluginConfigurationDraft,
  pluginConfigurationEditorModel,
  validatePluginConfigurationDraft,
} from './pluginConfigurationModel';

const mountId = parsePluginMountId('0190f5fe-7c00-7a00-8000-000000000021');
const artifactId = parsePluginArtifactId('0190f5fe-7c00-7a00-8000-000000000022');
const schemaDigest = 'd'.repeat(64);

const detail = (
  schema: unknown,
  values: Record<string, unknown> = {},
  credentialSlots: PluginDetail['credential_slots'] = []
): PluginDetail => ({
  summary: {
    mount_id: mountId,
    mount_revision: 11,
    display_name: 'Configuration Test Plugin',
    lifecycle: 'enabled',
    current: {
      package_id: 'dev.nomifun.configuration-test',
      package_version: '1.2.3',
      artifact_id: artifactId,
      artifact_digest: 'a'.repeat(64),
      manifest_digest: 'b'.repeat(64),
    },
    contribution_count: 1,
    updated_at_ms: 1,
  },
  capabilities: [],
  config_schema: { schema_digest: schemaDigest, schema },
  config: {
    config_revision: 5,
    schema_digest: schemaDigest,
    values,
    valid: true,
    validation_errors: [],
  },
  credential_bindings_revision: 7,
  credential_slots: credentialSlots,
  retained_data: false,
});

const supportedSchema = {
  type: 'object',
  additionalProperties: false,
  required: ['endpoint', 'retries'],
  properties: {
    endpoint: {
      type: 'string',
      title: 'Endpoint',
      minLength: 4,
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
};

describe('Plugin configuration editor model', () => {
  test('builds the supported primitive and enum field subset', () => {
    const plugin = detail(supportedSchema, {
      endpoint: 'https://old.example.test',
      retries: 2,
      enabled: false,
      mode: 'fast',
    });
    const editor = pluginConfigurationEditorModel(plugin);
    const draft = createPluginConfigurationDraft(plugin, editor);

    expect(editor.schemaIssues).toEqual([]);
    expect(editor.fields.map(({ key, kind }) => ({ key, kind }))).toEqual([
      { key: 'endpoint', kind: 'string' },
      { key: 'retries', kind: 'integer' },
      { key: 'enabled', kind: 'boolean' },
      { key: 'mode', kind: 'enum' },
    ]);
    expect(draft).toEqual({
      values: {
        endpoint: 'https://old.example.test',
        retries: 2,
        enabled: false,
        mode: 'fast',
      },
      credentials: {},
    });
  });

  test('builds one exact configure request and replaces the complete binding set', () => {
    const plugin = detail(
      supportedSchema,
      {
        endpoint: 'https://old.example.test',
        retries: 2,
        mode: 'fast',
      },
      [
        {
          slot_key: 'api_key',
          display_name: 'API key',
          required: true,
          status: 'bound',
          credential_id: 'credential://existing',
        },
        {
          slot_key: 'telemetry',
          display_name: 'Telemetry',
          required: false,
          status: 'bound',
          credential_id: 'credential://telemetry',
        },
        {
          slot_key: 'signing',
          display_name: 'Signing',
          required: true,
          status: 'unbound',
        },
      ]
    );
    const editor = pluginConfigurationEditorModel(plugin);
    const draft = createPluginConfigurationDraft(plugin, editor);
    draft.values.endpoint = 'https://new.example.test';
    draft.values.retries = 4;
    draft.values.mode = 'careful';
    draft.credentials.telemetry.action = 'unbind';
    draft.credentials.signing = {
      action: 'bind',
      credentialId: '  credential://signing  ',
    };

    expect(validatePluginConfigurationDraft(plugin, editor, draft)).toEqual([]);
    expect(configurePluginRequest(plugin, editor, draft)).toEqual({
      mount_id: mountId,
      expected_mount_revision: 11,
      expected_current_target_digest: 'a'.repeat(64),
      expected_config_revision: 5,
      expected_schema_digest: schemaDigest,
      values: {
        endpoint: 'https://new.example.test',
        retries: 4,
        mode: 'careful',
      },
      credential_bindings: {
        api_key: 'credential://existing',
        telemetry: null,
        signing: 'credential://signing',
      },
      expected_credential_bindings_revision: 7,
    });
  });

  test('validates required values, numeric limits, and required Credential bindings', () => {
    const plugin = detail(supportedSchema, {}, [
      {
        slot_key: 'api_key',
        display_name: 'API key',
        required: true,
        status: 'unbound',
      },
    ]);
    const editor = pluginConfigurationEditorModel(plugin);
    const draft = createPluginConfigurationDraft(plugin, editor);
    draft.values.endpoint = 'x';
    draft.values.retries = 10;

    expect(
      validatePluginConfigurationDraft(plugin, editor, draft).map(
        ({ code, key }) => `${key}:${code}`
      )
    ).toEqual([
      'endpoint:min_length',
      'retries:maximum',
      'api_key:credential_id_required',
    ]);
  });

  test('blocks marked and suspicious secret config without copying or returning values', () => {
    const sensitiveValues = {
      password: 'legacy-password-value',
      transport: 'legacy-write-only-value',
      auth_field: 'legacy-password-format-value',
      legacy_secret: 'legacy-extension-value',
      access_token: 'legacy-suspicious-name-value',
    };
    const plugin = detail(
      {
        type: 'object',
        additionalProperties: false,
        properties: {
          endpoint: {
            type: 'string',
            title: 'Endpoint',
          },
          password: {
            type: 'string',
          },
          transport: {
            type: 'string',
            writeOnly: true,
          },
          auth_field: {
            type: 'string',
            format: 'password',
          },
          legacy_secret: {
            type: 'string',
            'x-secret': true,
          },
          access_token: {
            type: 'string',
          },
        },
      },
      {
        endpoint: 'https://safe.example.test',
        ...sensitiveValues,
      }
    );
    const editor = pluginConfigurationEditorModel(plugin);
    const draft = createPluginConfigurationDraft(plugin, editor);

    expect(editor.canSubmit).toBe(false);
    expect(editor.fields.map((field) => field.key)).toEqual(['endpoint']);
    expect(
      editor.schemaIssues.filter(
        (issue) => issue.code === 'secret_config_unsupported'
      ).length
    ).toBe(5);
    expect(draft.values).toEqual({
      endpoint: 'https://safe.example.test',
    });
    const serializedDraft = JSON.stringify(draft);
    for (const value of Object.values(sensitiveValues)) {
      expect(serializedDraft.includes(value)).toBe(false);
    }

    let request: unknown;
    let error: unknown;
    try {
      request = configurePluginRequest(plugin, editor, draft);
    } catch (caught) {
      error = caught;
    }
    expect(request).toBeUndefined();
    expect(error instanceof Error).toBe(true);
    expect((error as Error).message.includes('not editable')).toBe(true);
  });

  test('blocks nested, dynamic, and unknown schema behavior instead of offering raw JSON', () => {
    const plugin = detail(
      {
        type: 'object',
        additionalProperties: true,
        properties: {
          nested: {
            type: 'object',
            properties: {
              value: { type: 'string' },
            },
          },
          patterned: {
            type: 'string',
            pattern: '^safe$',
          },
        },
      },
      {}
    );
    const editor = pluginConfigurationEditorModel(plugin);

    expect(editor.canSubmit).toBe(false);
    const issueCodes = editor.schemaIssues.map((issue) => issue.code);
    expect(issueCodes.includes('dynamic_properties_unsupported')).toBe(true);
    expect(issueCodes.includes('field_type_unsupported')).toBe(true);
    expect(
      editor.schemaIssues.some(
        (issue) =>
          issue.code === 'unsupported_keyword' &&
          issue.path === 'properties.patterned' &&
          issue.keyword === 'pattern'
      )
    ).toBe(true);
    let error: unknown;
    try {
      configurePluginRequest(
        plugin,
        editor,
        createPluginConfigurationDraft(plugin, editor)
      );
    } catch (caught) {
      error = caught;
    }
    expect(error instanceof Error).toBe(true);
    expect((error as Error).message.includes('not editable')).toBe(true);
  });
});
