/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { MODEL_TASK_ORDER } from '@/common/modelCapabilities';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import { createInstance } from 'i18next';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import {
  emptyCapabilityDraft,
  type ModelDefinitionDraft,
  type ModelProtocolManifest,
  type ModelProtocolManifestMap,
  type CapabilityValidationResult,
} from './providerModelAdvanced';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings } } },
  interpolation: { escapeValue: false },
});

const manifest = (task: ModelTask): ModelProtocolManifest => ({
  tasks: [...MODEL_TASK_ORDER],
  preset: 'StepFun',
  platform: 'stepfun',
  requested_task: task,
  platform_default_base_url: 'https://api.stepfun.com/v1',
  requires_user_input: false,
  default_auth_scheme: 'bearer',
  auth_schemes: [{ scheme: 'bearer', parameterized: false }],
  recommendation: {
    protocol_id: `stepfun.${task}`,
    connection_role: 'default',
    default_base_url: 'https://api.stepfun.com/v1',
    default_auth_scheme: 'bearer',
    base_url_override_required: false,
  },
  protocols: [
    {
      protocol_id: `stepfun.${task}`,
      supported_tasks: [task],
      executor: task === 'realtime_conversation' ? 'realtime_session' : 'model_invoke',
      transport: task === 'realtime_conversation' ? 'websocket' : 'http',
      allowed_auth_schemes: ['bearer'],
      scopes: ['native'],
      platforms: ['stepfun'],
      default_connections: [
        {
          preset: 'StepFun',
          platform: 'stepfun',
          connection_role: null,
          connection_label: null,
          base_url: 'https://api.stepfun.com/v1',
          auth_scheme: 'bearer',
          requires_credentials: false,
        },
      ],
      endpoints: [
        {
          task,
          field: task === 'realtime_conversation' ? 'realtime_endpoint' : 'endpoint',
          purpose: task === 'realtime_conversation' ? 'session' : 'submit',
          method: task === 'realtime_conversation' ? null : 'POST',
          default_value:
            task === 'realtime_conversation' ? 'wss://api.stepfun.com/v1/realtime' : `/v1/${task}`,
          allowed_placeholders: [],
          required_placeholders: [],
          editable: true,
        },
        ...(task === 'video_generation'
          ? [
              {
                task,
                field: 'content_endpoint',
                purpose: 'content' as const,
                method: 'GET',
                default_value: '/v1/videos/{id}/content',
                allowed_placeholders: ['id'],
                required_placeholders: ['id'],
                editable: true,
              },
            ]
          : []),
      ],
    },
  ],
});

const manifests: ModelProtocolManifestMap = Object.fromEntries(
  MODEL_TASK_ORDER.map((task) => [task, manifest(task)])
);

const render = (
  value: ModelDefinitionDraft,
  manifestMap: ModelProtocolManifestMap = manifests,
  providerAuthScheme = 'bearer',
  validationErrors: CapabilityValidationResult['errors'] = [],
  editorProps: Partial<React.ComponentProps<typeof ModelDefinitionEditor>> = {},
): string =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <ModelDefinitionEditor
        value={value}
        onChange={() => undefined}
        providerBaseUrl='https://api.stepfun.com/v1'
        providerAuthScheme={providerAuthScheme}
        manifests={manifestMap}
        validationErrors={validationErrors}
        {...editorProps}
      />
    </I18nextProvider>
  );

describe('unified model definition editor rendering and interactions', () => {
  test('keeps all nine capabilities intact when editing an existing model', () => {
    const definition: ModelDefinitionDraft = {
      model: 'user-entered/model-not-in-catalog',
      capabilities: MODEL_TASK_ORDER.map((task) => ({
        ...emptyCapabilityDraft(task),
        protocol: `stepfun.${task}`,
      })),
    };
    const html = render(definition, manifests, 'bearer', [], {
      modelReadOnly: true,
    });

    for (const task of MODEL_TASK_ORDER) {
      expect(html.includes(`data-capability-card="${task}"`)).toBe(true);
    }
    expect((html.match(/data-capability-card=/g) ?? []).length).toBe(9);
    expect((html.match(/data-remove-model-task=/g) ?? []).length).toBe(9);
    expect(html.includes('value="user-entered/model-not-in-catalog"')).toBe(true);
    expect(html.includes('data-readonly-model-id="true"')).toBe(true);
    expect(html.includes('data-primary-model-task-picker')).toBe(false);
    expect(html.includes('data-unified-model-input')).toBe(false);
    const chatHeader = html.slice(
      html.indexOf('data-capability-card-header="chat"'),
      html.indexOf('data-capability-details="chat"')
    );
    expect(chatHeader.indexOf('</button>')).toBeLessThan(
      chatHeader.indexOf('data-remove-model-task="chat"')
    );
    expect(html.includes('https://api.stepfun.com/v1')).toBe(true);
    expect(html.includes('data-endpoint-field="content_endpoint"')).toBe(true);
    expect(html.includes('/v1/videos/{id}/content')).toBe(true);
  });

  test('puts the model type before one unified catalog and free-text model input', () => {
    const html = render({ model: '', capabilities: [] }, manifests, 'bearer', [], {
      catalogSuggestions: [
        {
          value: 'chat-model',
          label: 'Chat model',
          tasks: ['chat'],
          traits: [],
        },
        {
          value: 'image-model',
          label: 'Image model',
          tasks: ['image_generation'],
          traits: [],
        },
      ],
    });

    expect(html.indexOf('data-primary-model-task-picker')).toBeLessThan(
      html.indexOf('data-unified-model-input')
    );
    expect(html.includes('data-model-catalog-picker')).toBe(false);
    expect(html.includes('disabled=""')).toBe(true);
    expect(html.includes('请先选择模型类型')).toBe(true);
  });

  test('only exposes catalog models compatible with the selected primary type', () => {
    const html = render(
      { model: '', capabilities: [emptyCapabilityDraft('image_generation')] },
      manifests,
      'bearer',
      [],
      {
        catalogSuggestions: [
          {
            value: 'chat-model',
            label: 'Chat model',
            tasks: ['chat'],
            traits: [],
          },
          {
            value: 'image-model',
            label: 'Image model',
            tasks: ['image_generation'],
            traits: [],
          },
          {
            value: 'taskless-model',
            label: 'Taskless model',
            tasks: [],
            traits: [],
          },
        ],
      }
    );

    expect(html.includes('data-filtered-catalog-count="1"')).toBe(true);
  });

  test('keeps ready capability details collapsed behind accessible advanced controls', () => {
    const definition: ModelDefinitionDraft = {
      model: 'step-ready',
      capabilities: ['chat', 'video_generation'].map((task) => ({
        ...emptyCapabilityDraft(task as ModelTask),
        protocol: `stepfun.${task}`,
      })),
    };
    const html = render(definition);

    expect((html.match(/data-capability-disclosure=/g) ?? []).length).toBe(2);
    expect(
      (html.match(/<button(?=[^>]*data-capability-disclosure=)(?=[^>]*aria-expanded="false")[^>]*>/g) ?? [])
        .length
    ).toBe(2);
    expect(html.includes('data-capability-summary="chat"')).toBe(true);
    expect(html.includes('高级配置')).toBe(true);
    expect(html.includes('默认配置已就绪')).toBe(true);
  });

  test('opens only the first capability with an actionable validation error', () => {
    const definition: ModelDefinitionDraft = {
      model: 'step-needs-attention',
      capabilities: ['chat', 'video_generation', 'speech_synthesis'].map((task) => ({
        ...emptyCapabilityDraft(task as ModelTask),
        protocol: `stepfun.${task}`,
      })),
    };
    const html = render(definition, manifests, 'bearer', [
      { task: 'video_generation', code: 'invalid_provider_params' },
      { task: 'speech_synthesis', code: 'connection_missing' },
    ]);

    expect(html.includes('data-capability-card="video_generation"')).toBe(true);
    expect(html.includes('data-capability-has-error="true"')).toBe(true);
    expect(
      (html.match(/<button(?=[^>]*data-capability-disclosure=)(?=[^>]*aria-expanded="true")[^>]*>/g) ?? [])
        .length
    ).toBe(1);
    expect(
      (html.match(/<button(?=[^>]*data-capability-disclosure=)(?=[^>]*aria-expanded="false")[^>]*>/g) ?? [])
        .length
    ).toBe(2);
    expect(html.includes('待处理 1 项')).toBe(true);
  });

  test('does not reveal transient protocol errors while defaults are being reconciled', () => {
    const html = render(
      {
        model: 'step-reconciling',
        capabilities: [emptyCapabilityDraft('chat')],
      },
      manifests,
      'bearer',
      [{ task: 'chat', code: 'protocol_required' }]
    );

    expect(html.includes('data-capability-has-error="false"')).toBe(true);
    expect(html.includes('aria-expanded="false"')).toBe(true);
    expect(html.includes('正在准备默认配置')).toBe(true);
  });

  test('keeps SDK capabilities free of Base URL and endpoint controls', () => {
    const bedrockManifest = manifest('chat');
    bedrockManifest.platform = 'bedrock';
    bedrockManifest.platform_default_base_url = null;
    bedrockManifest.default_auth_scheme = 'bedrock';
    bedrockManifest.auth_schemes = [{ scheme: 'bedrock', parameterized: false }];
    bedrockManifest.recommendation!.protocol_id = 'bedrock.anthropic_messages';
    bedrockManifest.recommendation!.default_base_url = null;
    bedrockManifest.recommendation!.default_auth_scheme = 'bedrock';
    bedrockManifest.protocols[0].protocol_id = 'bedrock.anthropic_messages';
    bedrockManifest.protocols[0].executor = 'agent';
    bedrockManifest.protocols[0].transport = 'sdk';
    bedrockManifest.protocols[0].allowed_auth_schemes = ['bedrock'];
    bedrockManifest.protocols[0].platforms = ['bedrock'];
    bedrockManifest.protocols[0].endpoints = [];

    const html = render(
      {
        model: 'anthropic.claude',
        capabilities: [
          { ...emptyCapabilityDraft('chat'), protocol: 'bedrock.anthropic_messages' },
        ],
      },
      { chat: bedrockManifest },
      'bedrock'
    );

    expect(html.includes('SDK')).toBe(true);
    expect(html.includes('data-effective-base-url=')).toBe(false);
    expect(html.includes('data-endpoint-field=')).toBe(false);
  });

  test('keeps a generic registered protocol selectable and shows compatibility and auth warnings', () => {
    const chatManifest = manifest('chat');
    const genericProtocol = {
      ...chatManifest.protocols[0],
      protocol_id: 'openai.chat_text',
      platforms: ['openai'],
      allowed_auth_schemes: ['header_key:<name>'],
      default_connections: [],
    };
    const html = render(
      {
        model: 'manual/generic-chat',
        capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: genericProtocol.protocol_id }],
      },
      {
        ...manifests,
        chat: { ...chatManifest, protocols: [...chatManifest.protocols, genericProtocol] },
      },
      'token'
    );

    expect(html.includes('data-generic-protocol-warning="true"')).toBe(true);
    expect(html.includes('data-protocol-auth-schemes="true"')).toBe(true);
    expect(html.includes('data-protocol-auth-incompatible="true"')).toBe(true);
    expect(html.includes('header_key:&lt;name&gt;')).toBe(true);
  });
});
