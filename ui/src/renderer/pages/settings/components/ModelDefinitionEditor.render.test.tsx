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
      root_shape: 'versioned_root' as const,
      supported_tasks: [task],
      executor: task === 'realtime_conversation' ? 'realtime_session' : 'model_invoke',
      transport: task === 'realtime_conversation' ? 'websocket' : 'http',
      requires_output_ceiling: false,
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
          root_shape: 'versioned_root' as const,
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
                root_shape: 'versioned_root' as const,
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

  /**
   * An inherited value must be rendered as a PLACEHOLDER, never as the input's
   * value. Rendering it as the value is what let a single keystroke promote
   * inheritance into a frozen `base_url_override` — after which editing the
   * provider's Base URL had no effect on this capability at all.
   */
  test('an un-overridden capability shows inherited values as placeholders, not values', () => {
    const definition: ModelDefinitionDraft = {
      model: 'm',
      capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'stepfun.chat' }],
    };
    const html = render(definition);

    // The provider URL appears, but only as a placeholder.
    expect(html.includes('placeholder="https://api.stepfun.com/v1"')).toBe(true);
    expect(html.includes('value="https://api.stepfun.com/v1"')).toBe(false);
    // Until the user opts in, the field cannot be typed into at all — the
    // structural guarantee that inheritance cannot be promoted by accident.
    expect(html.includes('data-base-url-override-toggle="chat"')).toBe(true);
    expect(html.includes('data-effective-base-url="https://api.stepfun.com/v1"')).toBe(true);

    // Same rule for the endpoint: the manifest default is a placeholder.
    expect(html.includes('placeholder="/v1/chat"')).toBe(true);
    expect(html.includes('value="/v1/chat"')).toBe(false);
    expect(html.includes('data-endpoint-override="false"')).toBe(true);
  });

  /**
   * The joined URL had no representation in the UI, so a doubled version
   * segment was invisible until it came back as a 404.
   */
  test('the capability card previews the exact request URL with the version seam collapsed', () => {
    const definition: ModelDefinitionDraft = {
      model: 'm',
      capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'stepfun.chat' }],
    };
    const html = render(definition);

    expect(html.includes('data-resolved-endpoint-url="endpoint"')).toBe(true);
    // base `…/v1` + template `/v1/chat` is exactly the pairing that used to
    // produce `/v1/v1/…`; the preview must show the collapsed truth.
    expect(html.includes('https://api.stepfun.com/v1/chat')).toBe(true);
    expect(html.includes('/v1/v1/')).toBe(false);
  });

  test('an explicit override is rendered as the input value and is editable', () => {
    const definition: ModelDefinitionDraft = {
      model: 'm',
      capabilities: [
        {
          ...emptyCapabilityDraft('chat'),
          protocol: 'stepfun.chat',
          baseUrlOverride: 'https://override.example.com/v1',
        },
      ],
    };
    const html = render(definition);

    expect(html.includes('value="https://override.example.com/v1"')).toBe(true);
    expect(html.includes('data-effective-base-url="https://override.example.com/v1"')).toBe(true);
  });

  test('nests the transport chain in the order it resolves', () => {
    const definition: ModelDefinitionDraft = {
      model: 'step-ready',
      capabilities: [
        { ...emptyCapabilityDraft('chat'), transportSource: 'persisted' as const, protocol: 'stepfun.chat' },
      ],
    };
    const html = render(definition);

    // One container for the whole chain...
    expect(html.includes('data-transport-group="chat"')).toBe(true);
    // ...and the connection profile that OWNS the URL comes before it, with the
    // override and the endpoints nested under it. This used to render inverted.
    const profileAt = html.indexOf('连接档案');
    const baseUrlAt = html.indexOf('data-transport-level="base-url"');
    const endpointsAt = html.indexOf('data-transport-level="endpoints"');
    expect(profileAt).toBeGreaterThan(-1);
    expect(profileAt).toBeLessThan(baseUrlAt);
    expect(baseUrlAt).toBeLessThan(endpointsAt);
    // Token ceilings are a separate concern, after the transport chain.
    expect(endpointsAt).toBeLessThan(html.indexOf('data-token-limits'));
  });

  test('names the missing config above the fold instead of only counting it', () => {
    const definition: ModelDefinitionDraft = {
      model: 'gpt-4o',
      capabilities: [emptyCapabilityDraft('chat')],
    };
    // A new-api-shaped manifest: protocols are selectable but nothing is
    // recommended, so the blank protocol is a settled error rather than a
    // transient "preparing defaults" state.
    const gatewayManifests: ModelProtocolManifestMap = {
      chat: { ...manifest('chat'), recommendation: null },
    };
    const html = render(definition, gatewayManifests, 'bearer', [
      { task: 'chat', code: 'protocol_required' },
    ]);

    // The sentence, not just "待处理 1 项".
    expect(html.includes('data-capability-error-list="chat"')).toBe(true);
    expect(html.includes('data-capability-error="protocol_required"')).toBe(true);
    expect(html.includes('请在调用配置里选择「兼容特殊调用方式」，再设置调用协议')).toBe(true);
    // Outside the collapsed details, so a collapsed card still explains itself.
    expect(html.indexOf('data-capability-error-list="chat"')).toBeLessThan(
      html.indexOf('data-capability-details="chat"')
    );
  });

  test('shows the declared task inside the picker and unlocks the model input', () => {
    // Regression: the task control used to reset to its placeholder after each
    // pick, so the form looked like it had discarded the choice and the user
    // never reached the model field — leaving save blocked on `model_required`.
    // The declared set is now the control's own value, rendered as tags in it.
    const html = render({ model: '', capabilities: [emptyCapabilityDraft('chat')] });

    expect(html.includes('data-model-task-picker')).toBe(true);
    // Arco renders a multi-select's value as tags inside the field.
    expect(html.includes('对话')).toBe(true);
    expect(html.includes('data-declared-tasks')).toBe(true);
    // One task is enough to unlock the model field; the placeholder must no
    // longer be the "pick a task first" prompt.
    expect(html.includes('请先在上方选择任务')).toBe(false);
    expect(html.includes('搜索目录模型，或直接输入官网模型 ID')).toBe(true);
  });

  test('puts the supported-task picker before one unified catalog and free-text model input', () => {
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

    // The task picker must exist with zero capabilities: it is the only control
    // that can create the first one, so gating it would deadlock the form.
    expect(html.includes('data-model-task-picker')).toBe(true);
    expect(html.indexOf('data-model-task-picker')).toBeLessThan(
      html.indexOf('data-unified-model-input')
    );
    expect(html.includes('data-model-catalog-picker')).toBe(false);
    expect(html.includes('data-primary-model-task-picker')).toBe(false);
    expect(html.includes('disabled=""')).toBe(true);
    expect(html.includes('请先在上方选择任务')).toBe(true);
  });

  test('keeps traits answerable without expanding a capability card', () => {
    const html = render({
      model: 'step-ready',
      capabilities: [
        { ...emptyCapabilityDraft('chat'), transportSource: 'persisted' as const, protocol: 'stepfun.chat' },
      ],
    });

    // Traits describe what the model can do — the same kind of question as the
    // task itself. They must sit outside the collapsed transport details.
    expect(html.includes('data-capability-traits="chat"')).toBe(true);
    expect(html.indexOf('data-capability-traits="chat"')).toBeLessThan(
      html.indexOf('data-capability-details="chat"')
    );
    expect(html.includes('data-capability-expanded="false"')).toBe(true);
  });

  test('groups both token ceilings under one heading', () => {
    const html = render({
      model: 'step-ready',
      capabilities: [
        { ...emptyCapabilityDraft('chat'), transportSource: 'persisted' as const, protocol: 'stepfun.chat' },
      ],
    });

    expect(html.includes('data-token-limits')).toBe(true);
    expect(html.includes('上下文窗口')).toBe(true);
    expect(html.includes('最大输出（tokens）')).toBe(true);
    expect(html.includes('data-output-limit-input')).toBe(true);
    expect(html.includes('未设置最大输出，将使用供应商默认值。')).toBe(true);
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
        transportSource: 'persisted' as const,
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
    expect(html.includes('查看调用配置')).toBe(true);
    expect(html.includes('默认配置已就绪')).toBe(true);
  });

  test('starts call configuration with a goal chooser instead of a flat full form', () => {
    const html = render({
      model: 'step-ready',
      capabilities: [
        {
          ...emptyCapabilityDraft('chat'),
          transportSource: 'persisted' as const,
          protocol: 'stepfun.chat',
        },
      ],
    });

    expect(html.includes('data-call-config-intents="chat"')).toBe(true);
    expect(html.includes('data-call-config-intent="connection"')).toBe(true);
    expect(html.includes('data-call-config-intent="limits"')).toBe(true);
    expect(html.includes('data-call-config-intent="protocol"')).toBe(true);
    expect(html.includes('data-call-config-intent="diagnostics"')).toBe(true);
    expect(html.includes('data-call-config-intent="recommended"')).toBe(true);
    expect(html.includes('你想调整什么？')).toBe(true);
    expect(html.includes('保持推荐配置，不做调整')).toBe(true);
    expect(html.includes('data-call-config-branch="recommended"')).toBe(true);
    expect(html.includes('data-call-config-branch="limits"')).toBe(true);
    expect(html.includes('data-call-config-branch="diagnostics"')).toBe(true);
  });

  test('opens only the first capability with an actionable validation error', () => {
    const definition: ModelDefinitionDraft = {
      model: 'step-needs-attention',
      capabilities: ['chat', 'video_generation', 'speech_synthesis'].map((task) => ({
        ...emptyCapabilityDraft(task as ModelTask),
        transportSource: 'persisted' as const,
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

  test('treats the backend-recommended Custom compatibility protocol as ready', () => {
    const chatManifest = manifest('chat');
    const customProtocol: ModelProtocolManifest['protocols'][number] = {
      ...chatManifest.protocols[0],
      protocol_id: 'openai.chat_text',
      scopes: ['official_compat', 'custom'],
      platforms: ['openai'],
      default_connections: [],
    };
    const customManifest: ModelProtocolManifest = {
      ...chatManifest,
      preset: 'custom',
      platform: 'custom',
      platform_default_base_url: null,
      requires_user_input: true,
      recommendation: {
        protocol_id: customProtocol.protocol_id,
        connection_role: null,
        default_base_url: null,
        default_auth_scheme: 'bearer',
        base_url_override_required: false,
      },
      protocols: [customProtocol],
    };
    const html = render(
      {
        model: 'gateway/model',
        capabilities: [
          {
            ...emptyCapabilityDraft('chat'),
            transportSource: 'recommendation',
            protocol: customProtocol.protocol_id,
          },
        ],
      },
      { chat: customManifest }
    );

    expect(html.includes('默认配置已就绪')).toBe(true);
    expect(html.includes('当前供应商推荐')).toBe(true);
    expect(html.includes('data-generic-protocol-warning="true"')).toBe(false);
  });

  /**
   * StepFun TTS rejects a request with no voice locally, so model management
   * has to offer the field. Before this control the only way to set one was to
   * hand-type `{"voice": …}` into the raw provider-params JSON.
   */
  test('offers a default-voice picker for speech synthesis, seeded from the persisted voice', () => {
    const html = render({
      model: 'step-tts-mini',
      capabilities: [
        {
          ...emptyCapabilityDraft('speech_synthesis'),
          protocol: 'stepfun.audio_speech',
          providerParamsJson: '{"voice":"cixingnansheng"}',
        },
      ],
    });

    expect(html.includes('默认音色')).toBe(true);
    // The persisted voice is shown as the current selection, not just a suggestion.
    expect(html.includes('cixingnansheng')).toBe(true);
  });

  test('hides the voice picker for non-TTS tasks and for protocols that ignore a provider-params voice', () => {
    const chatHtml = render({
      model: 'step-3.7-flash',
      capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'stepfun.chat' }],
    });
    expect(chatHtml.includes('默认音色')).toBe(false);

    // deepgram.speak_rest encodes the voice in the model id, so a separate
    // voice field would be a second, contradictory source of truth.
    const deepgramHtml = render({
      model: 'aura-asteria-en',
      capabilities: [
        { ...emptyCapabilityDraft('speech_synthesis'), protocol: 'deepgram.speak_rest' },
      ],
    });
    expect(deepgramHtml.includes('默认音色')).toBe(false);

    // These adapters read a differently-named field (voice_setting.voice_id,
    // speaker, voice_id) or overwrite provider params with a typed default, so
    // a saved `voice` key would silently never take effect.
    for (const protocol of [
      'minimax.t2a',
      'volc.tts_v3',
      'xai.tts',
      'mimo.chat_tts',
      'openai.audio_speech',
    ]) {
      const html = render({
        model: 'some-tts-model',
        capabilities: [{ ...emptyCapabilityDraft('speech_synthesis'), protocol }],
      });
      expect(html.includes('默认音色')).toBe(false);
    }
  });

  test('shows Responses round chaining only for openai.responses and before raw provider params', () => {
    const responsesManifest = manifest('chat');
    responsesManifest.recommendation!.protocol_id = 'openai.responses';
    responsesManifest.protocols[0] = {
      ...responsesManifest.protocols[0],
      protocol_id: 'openai.responses',
      platforms: ['openai'],
    };
    const html = render(
      {
        model: 'gpt-5.4',
        capabilities: [
          {
            ...emptyCapabilityDraft('chat'),
            protocol: 'openai.responses',
            providerParamsJson: '{"chain_rounds":true,"temperature":0.2}',
          },
        ],
      },
      { ...manifests, chat: responsesManifest }
    );

    const chainRounds = html.indexOf('data-chain-rounds-control="true"');
    const rawParams = html.indexOf('data-provider-params-json="true"');
    expect(chainRounds).toBeGreaterThan(-1);
    expect(rawParams).toBeGreaterThan(chainRounds);
    expect(html.includes('data-chain-rounds-enabled="true"')).toBe(true);
    expect(html.includes('store: true')).toBe(true);
    expect(html.includes('至少 30 天')).toBe(true);
    expect(html.includes('不会减少计费的输入 tokens')).toBe(true);

    const otherProtocol = render({
      model: 'step-3.7-flash',
      capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'stepfun.chat' }],
    });
    expect(otherProtocol.includes('data-chain-rounds-control')).toBe(false);
  });

  test('disables the Responses round-chaining checkbox while raw JSON is invalid', () => {
    const responsesManifest = manifest('chat');
    responsesManifest.recommendation!.protocol_id = 'openai.responses';
    responsesManifest.protocols[0] = {
      ...responsesManifest.protocols[0],
      protocol_id: 'openai.responses',
      platforms: ['openai'],
    };
    const html = render(
      {
        model: 'gpt-5.4',
        capabilities: [
          {
            ...emptyCapabilityDraft('chat'),
            protocol: 'openai.responses',
            providerParamsJson: ' {"chain_rounds": tru',
          },
        ],
      },
      { ...manifests, chat: responsesManifest }
    );
    const start = html.indexOf('data-chain-rounds-control="true"');
    const end = html.indexOf('data-provider-params-json="true"', start);
    const control = html.slice(start, end);

    expect(start).toBeGreaterThan(-1);
    expect(control.includes('data-chain-rounds-json-valid="false"')).toBe(true);
    expect(control.includes('disabled=""')).toBe(true);
    expect(control.includes('修正后才能更改此选项')).toBe(true);
  });
});
