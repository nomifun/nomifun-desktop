/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import {
  addCapabilityTask,
  applyCatalogSuggestionForTask,
  catalogSuggestionsForTask,
  capabilityDraftFromResponse,
  capabilityInputsFromDefinition,
  changeCapabilityProtocol,
  effectiveBaseUrl,
  emptyCapabilityDraft,
  isProtocolAuthSchemeAllowed,
  isDuplicateModelId,
  normalizeModelId,
  patchCapabilityDraft,
  providerParamChainRounds,
  providerParamVoice,
  reconcileCapabilityRecommendations,
  removeCapabilityTask,
  resolveModelInputChange,
  requiresCrossOriginConsent,
  withProviderParamVoice,
  withProviderParamChainRounds,
  validateModelDefinition,
  type ModelCapabilityDraft,
  type ModelProtocolManifest,
} from './providerModelAdvanced';

const manifest = (
  task: ModelTask,
  protocolId: string,
  defaultBaseUrl = 'https://api.stepfun.com/v1'
): ModelProtocolManifest => ({
  tasks: [task],
  preset: 'stepfun',
  platform: 'stepfun',
  platform_default_base_url: 'https://api.stepfun.com/v1',
  default_auth_scheme: 'bearer',
  auth_schemes: [{ scheme: 'bearer', parameterized: false }],
  requires_user_input: false,
  requested_task: task,
  recommendation: {
    protocol_id: protocolId,
    connection_role: 'default',
    default_base_url: defaultBaseUrl,
    default_auth_scheme: 'bearer',
    base_url_override_required: false,
  },
  protocols: [
    {
      protocol_id: protocolId,
      root_shape: 'versioned_root' as const,
      supported_tasks: [task],
      executor: 'model_invoke',
      transport: task === 'realtime_conversation' ? 'websocket' : 'http',
      requires_output_ceiling: false,
      allowed_auth_schemes: ['bearer'],
      scopes: [],
      platforms: ['stepfun'],
      default_connections: [
        {
          preset: 'stepfun',
          platform: 'stepfun',
          connection_role: null,
          connection_label: null,
          base_url: defaultBaseUrl,
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
            task === 'realtime_conversation' ? 'wss://api.stepfun.com/v1/realtime' : '/audio/speech',
          root_shape: 'versioned_root' as const,
          allowed_placeholders: [],
          required_placeholders: [],
          editable: true,
        },
      ],
    },
  ],
});

describe('model definition capability selection', () => {
  test('keeps free-text changes separate from the catalog onChange then onSelect event sequence', () => {
    let definition = { model: '', capabilities: [emptyCapabilityDraft('chat')] };

    const manualInput = resolveModelInputChange('vendor/custom-chat');
    if (manualInput !== undefined) definition = { ...definition, model: manualInput };
    expect(definition.model).toBe('vendor/custom-chat');

    const catalogInputChange = resolveModelInputChange('catalog/chat', { value: 'catalog/chat' });
    if (catalogInputChange !== undefined) definition = { ...definition, model: catalogInputChange };
    expect(definition.model).toBe('vendor/custom-chat');

    definition = applyCatalogSuggestionForTask(
      definition,
      { model: 'catalog/chat', tasks: ['chat', 'embedding'], traits: ['reasoning'] },
      'chat'
    );
    expect(definition).toEqual({
      model: 'catalog/chat',
      capabilities: [{ ...emptyCapabilityDraft('chat'), traits: ['reasoning'] }],
    });
  });

  test('filters catalog suggestions by the selected task without treating taskless models as universal', () => {
    const suggestions = [
      { model: 'chat-only', tasks: ['chat'] as ModelTask[], traits: [] },
      { model: 'shared', tasks: ['chat', 'speech_synthesis'] as ModelTask[], traits: [] },
      { model: 'unknown', tasks: [] as ModelTask[], traits: [] },
    ];

    expect(catalogSuggestionsForTask(suggestions, 'speech_synthesis').map((item) => item.model)).toEqual([
      'shared',
    ]);
    expect(catalogSuggestionsForTask(suggestions, undefined)).toEqual([]);
  });

  test('adopting a catalog model preserves every other configured task', () => {
    const oldChat: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('chat'),
      traits: ['reasoning'],
      protocol: 'old.chat',
      endpoint: '/old/chat',
      providerParamsJson: '{"old":true}',
    };
    const applied = applyCatalogSuggestionForTask(
      { model: 'old/model', capabilities: [oldChat] },
      {
        model: 'catalog/model',
        tasks: ['speech_synthesis', 'chat', 'realtime_conversation', 'chat'],
        traits: [
          'web_search',
          'realtime',
          'audio_output',
          'vision_input',
          'streaming',
          'audio_input',
        ],
      },
      'speech_synthesis'
    );

    expect(applied.model).toBe('catalog/model');
    // The catalog is advisory. It may add the task it was chosen for; it may
    // never discard a task the user already configured.
    expect(applied.capabilities).toEqual([oldChat, emptyCapabilityDraft('speech_synthesis')]);
    expect(applied.capabilities[0]).toBe(oldChat);
  });

  test('adopting a catalog model keeps the chosen task transport and only refreshes its traits', () => {
    const configuredChat: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('chat'),
      traits: ['audio_input'],
      protocol: 'openai.chat_text',
      endpoint: '/chat/completions',
      providerParamsJson: '{"temperature":0.2}',
    };

    expect(
      applyCatalogSuggestionForTask(
        { model: 'old/model', capabilities: [configuredChat] },
        { model: 'catalog/chat', tasks: ['chat'], traits: ['reasoning', 'vision_input'] },
        'chat'
      )
    ).toEqual({
      model: 'catalog/chat',
      capabilities: [{ ...configuredChat, traits: ['vision_input', 'reasoning'] }],
    });
  });

  test('does not touch traits when the selected task is absent from the entry', () => {
    const oldSpeech: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('speech_synthesis'),
      traits: ['audio_output'],
      protocol: 'old.speech',
      endpoint: '/old/speech',
    };

    expect(
      applyCatalogSuggestionForTask(
        { model: 'old/model', capabilities: [oldSpeech] },
        { model: 'catalog/unknown', tasks: [], traits: ['audio_output'] },
        'speech_synthesis'
      )
    ).toEqual({ model: 'catalog/unknown', capabilities: [oldSpeech] });
  });

  test('adds and removes task capabilities without changing unrelated drafts', () => {
    const chat: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('chat'),
      traits: ['vision_input'],
      protocol: 'openai.chat_text',
      endpoint: '/chat',
    };
    const withSpeech = addCapabilityTask([chat], 'speech_synthesis');

    expect(withSpeech).toEqual([chat, emptyCapabilityDraft('speech_synthesis')]);
    expect(withSpeech[0]).toBe(chat);
    expect(addCapabilityTask(withSpeech, 'chat')).toEqual(withSpeech);
    expect(removeCapabilityTask(withSpeech, 'speech_synthesis')).toEqual([chat]);
    expect(removeCapabilityTask(withSpeech, 'embedding')).toEqual(withSpeech);
  });

  test('applies recommendations only to blank transport and preserves user-owned transport', () => {
    const tts = emptyCapabilityDraft('speech_synthesis');
    const realtime = patchCapabilityDraft(emptyCapabilityDraft('realtime_conversation'), {
      protocol: 'manual.realtime',
    });
    const manifests = {
      speech_synthesis: manifest('speech_synthesis', 'stepfun.audio_speech'),
      realtime_conversation: manifest('realtime_conversation', 'stepfun.realtime_s2s'),
    };

    expect(reconcileCapabilityRecommendations([tts, realtime], manifests)).toMatchObject([
      { task: 'speech_synthesis', protocol: 'stepfun.audio_speech', connectionRole: 'default' },
      { task: 'realtime_conversation', protocol: 'manual.realtime', connectionRole: 'default' },
    ]);
  });

  test('replaces a previous automatic recommendation when the selected model recommendation changes', () => {
    const firstManifest = manifest('chat', 'openai.chat_text');
    firstManifest.recommendation!.base_url_override_required = true;
    firstManifest.recommendation!.default_base_url = 'https://first.example/v1';
    const [first] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: firstManifest,
    });
    expect(first).toMatchObject({
      protocol: 'openai.chat_text',
      baseUrlOverride: 'https://first.example/v1',
      transportSource: 'recommendation',
    });

    const secondManifest = manifest('chat', 'anthropic.messages', 'https://second.example');
    secondManifest.recommendation!.base_url_override_required = false;
    const [second] = reconcileCapabilityRecommendations([first], { chat: secondManifest });
    expect(second).toMatchObject({
      protocol: 'anthropic.messages',
      connectionRole: 'default',
      baseUrlOverride: '',
      endpoint: '',
      providerParamsJson: '',
      transportSource: 'recommendation',
    });
  });

  test('never replaces user-edited or persisted transport when recommendations refresh', () => {
    const user = patchCapabilityDraft(emptyCapabilityDraft('chat'), {
      protocol: 'manual.chat',
      connectionRole: 'custom_api',
      endpoint: '/manual',
    });
    const persisted = capabilityDraftFromResponse({
      task: 'chat',
      protocol: 'stored.chat',
      connection_role: 'default',
      endpoint: '/stored',
    });
    const recommendation = { chat: manifest('chat', 'openai.chat_text') };

    const reconciled = reconcileCapabilityRecommendations([user, persisted], recommendation);
    expect(reconciled[0]).toBe(user);
    expect(reconciled[1]).toBe(persisted);
  });

  test('clears only recommendation-owned transport when a model no longer has a safe default', () => {
    const [recommended] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: manifest('chat', 'openai.chat_text'),
    });
    expect(reconcileCapabilityRecommendations([recommended], {})[0]).toBe(recommended);

    const withoutRecommendation = manifest('chat', 'openai.chat_text');
    withoutRecommendation.recommendation = null;

    expect(
      reconcileCapabilityRecommendations([recommended], { chat: withoutRecommendation })[0]
    ).toEqual(emptyCapabilityDraft('chat'));
  });

  test('keeps an automatic protocol after the user explicitly confirms the same option', () => {
    const taskManifest = manifest('chat', 'openai.chat_text');
    const [recommended] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: taskManifest,
    });
    const confirmed = changeCapabilityProtocol(recommended, recommended.protocol, taskManifest);
    expect(confirmed.transportSource).toBe('user');

    taskManifest.recommendation = null;
    expect(reconcileCapabilityRecommendations([confirmed], { chat: taskManifest })[0]).toBe(
      confirmed
    );
  });

  test('persists required task base overrides and keeps named-role base URLs out of capabilities', () => {
    const gemini = manifest('chat', 'openai.chat_text', 'https://generativelanguage.googleapis.com/v1beta/openai');
    gemini.recommendation!.base_url_override_required = true;
    const [chat] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], { chat: gemini });
    expect(chat.baseUrlOverride).toBe('https://generativelanguage.googleapis.com/v1beta/openai');

    const ark = manifest('speech_synthesis', 'volc.tts_v3', 'https://openspeech.bytedance.com');
    ark.recommendation!.connection_role = 'voice';
    ark.recommendation!.base_url_override_required = false;
    ark.protocols[0].default_connections[0] = {
      preset: 'Ark',
      platform: 'ark',
      connection_role: 'voice',
      connection_label: 'Volcengine Voice',
      base_url: 'https://openspeech.bytedance.com',
      auth_scheme: 'volc_voice',
      requires_credentials: true,
    };
    const [speech] = reconcileCapabilityRecommendations([emptyCapabilityDraft('speech_synthesis')], {
      speech_synthesis: ark,
    });
    expect(speech.connectionRole).toBe('voice');
    expect(speech.baseUrlOverride).toBe('');
    expect(
      validateModelDefinition(
        { model: 'doubao-tts', capabilities: [speech] },
        { speech_synthesis: ark },
        'https://ark.cn-beijing.volces.com/api/v3'
      ).errors
    ).toEqual([{ task: 'speech_synthesis', code: 'connection_missing' }]);
    expect(
      validateModelDefinition(
        { model: 'doubao-tts', capabilities: [speech] },
        { speech_synthesis: ark },
        'https://ark.cn-beijing.volces.com/api/v3',
        [],
        [],
        ['voice']
      ).valid
    ).toBe(true);
  });

  test('switching to a non-recommended protocol clears adapter-owned transport state atomically', () => {
    const taskManifest = manifest('speech_synthesis', 'stepfun.audio_speech');
    taskManifest.protocols.push({
      ...taskManifest.protocols[0],
      protocol_id: 'openai.audio_speech',
      platforms: ['openai'],
      default_connections: [],
      endpoints: [],
    });
    const current: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('speech_synthesis'),
      traits: ['audio_output'],
      protocol: 'stepfun.audio_speech',
      connectionRole: 'voice',
      baseUrlOverride: 'https://old.example/v1',
      endpoint: '/old-speech',
      pollEndpoint: '/old-poll',
      contentEndpoint: '/old-content',
      realtimeEndpoint: 'wss://old.example/realtime',
      allowCrossOriginCredentials: true,
      providerParamsJson: '{"voice":"alloy"}',
      contextLimit: 32_000,
      outputLimit: 8_192,
    };

    expect(changeCapabilityProtocol(current, current.protocol, taskManifest)).toEqual({
      ...current,
      transportSource: 'user',
    });
    const changed = changeCapabilityProtocol(current, 'openai.audio_speech', taskManifest);
    expect(changed).toEqual({
      ...current,
      transportSource: 'user',
      protocol: 'openai.audio_speech',
      connectionRole: 'default',
      baseUrlOverride: '',
      endpoint: '',
      pollEndpoint: '',
      contentEndpoint: '',
      realtimeEndpoint: '',
      allowCrossOriginCredentials: false,
      providerParamsJson: '',
    });
    expect(
      validateModelDefinition(
        { model: 'custom-audio', capabilities: [changed] },
        { speech_synthesis: taskManifest },
        'https://api.stepfun.com/v1'
      ).valid
    ).toBe(true);
  });

  test('switching back to the recommendation reapplies its role and required URL override', () => {
    const taskManifest = manifest(
      'chat',
      'openai.chat_text',
      'https://generativelanguage.googleapis.com/v1beta/openai'
    );
    taskManifest.recommendation!.base_url_override_required = true;
    const current = {
      ...emptyCapabilityDraft('chat'),
      protocol: 'anthropic.messages',
      endpoint: '/v1/messages',
      allowCrossOriginCredentials: true,
    };

    expect(changeCapabilityProtocol(current, 'openai.chat_text', taskManifest)).toMatchObject({
      protocol: 'openai.chat_text',
      connectionRole: 'default',
      baseUrlOverride: 'https://generativelanguage.googleapis.com/v1beta/openai',
      endpoint: '',
      allowCrossOriginCredentials: false,
    });
  });
});

describe('capability validation and serialization', () => {
  test('requires a registered provider-by-task adapter while keeping the task selectable', () => {
    const definition = { model: 'step-audio-latest', capabilities: [emptyCapabilityDraft('speech_synthesis')] };
    expect(validateModelDefinition(definition, {}, 'https://api.stepfun.com/v1')).toEqual({
      valid: false,
      errors: [{ task: 'speech_synthesis', code: 'manifest_unavailable' }],
    });

    const recommended = reconcileCapabilityRecommendations(definition.capabilities, {
      speech_synthesis: manifest('speech_synthesis', 'stepfun.audio_speech'),
    });
    expect(
      validateModelDefinition(
        { ...definition, capabilities: recommended },
        { speech_synthesis: manifest('speech_synthesis', 'stepfun.audio_speech') },
        'https://api.stepfun.com/v1'
      ).valid
    ).toBe(true);
  });

  test('validates exact and parameterized authentication schemes from the protocol descriptor', () => {
    expect(isProtocolAuthSchemeAllowed('bearer', ['bearer'])).toBe(true);
    expect(isProtocolAuthSchemeAllowed('token', ['bearer'])).toBe(false);
    expect(isProtocolAuthSchemeAllowed('header_key:x-api-key', ['header_key:<name>'])).toBe(true);
    expect(isProtocolAuthSchemeAllowed('query_key:key', ['query_key:<param>'])).toBe(true);
    expect(isProtocolAuthSchemeAllowed('header_key:', ['header_key:<name>'])).toBe(false);

    const chatManifest = manifest('chat', 'openai.chat_text');
    const chat = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: chatManifest,
    })[0];
    expect(
      validateModelDefinition(
        { model: 'step-chat', capabilities: [chat] },
        { chat: chatManifest },
        'https://api.stepfun.com/v1',
        [],
        [],
        [],
        'token'
      ).errors.some(
        (error) => error.task === 'chat' && error.code === 'auth_scheme_incompatible'
      )
    ).toBe(true);
  });

  test('requires an effective base URL for a resolvable capability connection', () => {
    const chatManifest = manifest('chat', 'openai.chat_text', '');
    const chat = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: chatManifest,
    })[0];

    expect(
      validateModelDefinition(
        { model: 'step-chat', capabilities: [chat] },
        { chat: chatManifest },
        ''
      ).errors.some((error) => error.task === 'chat' && error.code === 'base_url_required')
    ).toBe(true);
  });

  test('does not require a Base URL for SDK-backed capabilities', () => {
    const bedrockManifest = manifest('chat', 'bedrock.anthropic_messages', '');
    bedrockManifest.platform = 'bedrock';
    bedrockManifest.platform_default_base_url = null;
    bedrockManifest.default_auth_scheme = 'bedrock';
    bedrockManifest.auth_schemes = [{ scheme: 'bedrock', parameterized: false }];
    bedrockManifest.recommendation!.default_base_url = null;
    bedrockManifest.recommendation!.default_auth_scheme = 'bedrock';
    bedrockManifest.protocols[0].executor = 'agent';
    bedrockManifest.protocols[0].transport = 'sdk';
    bedrockManifest.protocols[0].allowed_auth_schemes = ['bedrock'];
    bedrockManifest.protocols[0].platforms = ['bedrock'];
    bedrockManifest.protocols[0].endpoints = [];
    const chat = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: bedrockManifest,
    })[0];

    const result = validateModelDefinition(
      { model: 'anthropic.claude', capabilities: [chat] },
      { chat: bedrockManifest },
      '',
      [],
      [],
      [],
      'bedrock'
    );

    expect(result.errors.some((error) => error.code === 'base_url_required')).toBe(false);
    expect(result.valid).toBe(true);
  });

  test('requires an output limit only when the selected protocol declares it mandatory', () => {
    const chatManifest = manifest('chat', 'anthropic.messages');
    chatManifest.protocols[0].requires_output_ceiling = true;
    const chat = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], {
      chat: chatManifest,
    })[0];

    expect(
      validateModelDefinition(
        { model: 'claude', capabilities: [chat] },
        { chat: chatManifest },
        'https://api.anthropic.com'
      ).errors.some(
        (error) => error.task === 'chat' && error.code === 'output_ceiling_required'
      )
    ).toBe(true);

    expect(
      validateModelDefinition(
        { model: 'claude', capabilities: [{ ...chat, outputLimit: 8_192 }] },
        { chat: chatManifest },
        'https://api.anthropic.com'
      ).errors.some(
        (error) => error.task === 'chat' && error.code === 'output_ceiling_required'
      )
    ).toBe(false);
  });

  test('serializes multiple capabilities as typed task records', () => {
    const definition = {
      model: 'step-audio-latest',
      capabilities: [
        {
          ...emptyCapabilityDraft('speech_synthesis'),
          protocol: 'stepfun.audio_speech',
          endpoint: '/v1/audio/speech',
          providerParamsJson: '{"voice":"cixingnansheng"}',
          contextLimit: 32000,
          outputLimit: 16384,
        },
        {
          ...emptyCapabilityDraft('realtime_conversation'),
          protocol: 'stepfun.realtime_s2s',
          realtimeEndpoint: 'wss://api.stepfun.com/v1/realtime',
        },
        {
          ...emptyCapabilityDraft('video_generation'),
          protocol: 'openai.video_generation',
          endpoint: '/v1/videos',
          contentEndpoint: '/v1/videos/{id}/content',
        },
      ],
    };

    const input = capabilityInputsFromDefinition(definition);
    expect(input).toEqual([
      {
        task: 'speech_synthesis',
        protocol: 'stepfun.audio_speech',
        connection_role: 'default',
        endpoint: '/v1/audio/speech',
        provider_params: { voice: 'cixingnansheng' },
        context_limit: 32000,
        output_limit: 16384,
      },
      {
        task: 'realtime_conversation',
        protocol: 'stepfun.realtime_s2s',
        connection_role: 'default',
        realtime_endpoint: 'wss://api.stepfun.com/v1/realtime',
      },
      {
        task: 'video_generation',
        protocol: 'openai.video_generation',
        connection_role: 'default',
        endpoint: '/v1/videos',
        content_endpoint: '/v1/videos/{id}/content',
      },
    ]);
  });

  test('round-trips one persisted capability into the typed editor draft', () => {
    expect(
      capabilityDraftFromResponse({
        task: 'speech_synthesis',
        traits: ['audio_output'],
        protocol: 'stepfun.audio_speech',
        connection_role: 'voice',
        base_url_override: 'https://voice.example/v1',
        endpoint: '/speech',
        allow_cross_origin_credentials: true,
        provider_params: { voice: 'alloy' },
        context_limit: 4096,
        output_limit: 8192,
      })
    ).toEqual({
      task: 'speech_synthesis',
      traits: ['audio_output'],
      transportSource: 'persisted',
      protocol: 'stepfun.audio_speech',
      connectionRole: 'voice',
      baseUrlOverride: 'https://voice.example/v1',
      endpoint: '/speech',
      pollEndpoint: '',
      contentEndpoint: '',
      realtimeEndpoint: '',
      allowCrossOriginCredentials: true,
      providerParamsJson: '{\n  "voice": "alloy"\n}',
      contextLimit: 4096,
      outputLimit: 8192,
    });
  });
});

describe('effective URL and credential consent', () => {
  test('shows only persisted provider, connection, or task URLs', () => {
    const taskManifest = manifest('speech_synthesis', 'stepfun.audio_speech');
    const inherited = { ...emptyCapabilityDraft('speech_synthesis'), protocol: 'stepfun.audio_speech' };
    expect(effectiveBaseUrl(inherited, taskManifest, 'https://provider.example/v1')).toBe(
      'https://provider.example/v1'
    );
    expect(
      effectiveBaseUrl(
        { ...inherited, baseUrlOverride: 'https://voice.example/v2' },
        taskManifest,
        'https://provider.example/v1'
      )
    ).toBe('https://voice.example/v2');
    expect(
      effectiveBaseUrl(
        { ...inherited, connectionRole: 'voice' },
        taskManifest,
        'https://provider.example/v1',
        [{ role: 'voice', base_url: 'https://stored-voice.example/v1', auth_scheme: 'volc_voice' }]
      )
    ).toBe('https://stored-voice.example/v1');
  });

  test('requires explicit consent only when credentials would leave the provider origin', () => {
    const taskManifest = manifest('speech_synthesis', 'stepfun.audio_speech');
    const sameOrigin = {
      ...emptyCapabilityDraft('speech_synthesis'),
      protocol: 'stepfun.audio_speech',
      endpoint: 'https://api.stepfun.com/v1/audio/speech',
    };
    expect(requiresCrossOriginConsent(sameOrigin, taskManifest, 'https://api.stepfun.com/v1')).toBe(false);
    expect(
      requiresCrossOriginConsent(
        { ...sameOrigin, endpoint: 'https://voice.example/v1/speech' },
        taskManifest,
        'https://api.stepfun.com/v1'
      )
    ).toBe(true);
  });

  test('uses the persisted named connection as the credential origin', () => {
    const taskManifest = manifest('video_generation', 'openai.video_generation');
    const connections = [
      { role: 'media', base_url: 'https://media.example/v1', auth_scheme: 'bearer' },
    ];
    const named = {
      ...emptyCapabilityDraft('video_generation'),
      protocol: 'openai.video_generation',
      connectionRole: 'media',
      contentEndpoint: 'https://media.example/v1/videos/123/content',
    };
    expect(
      requiresCrossOriginConsent(named, taskManifest, 'https://provider.example/v1', connections)
    ).toBe(false);
    const crossOriginNamed = {
      ...named,
      contentEndpoint: 'https://cdn.example/videos/123/content',
    };
    expect(
      requiresCrossOriginConsent(
        crossOriginNamed,
        taskManifest,
        'https://provider.example/v1',
        connections
      )
    ).toBe(true);
    expect(
      validateModelDefinition(
        { model: 'video-model', capabilities: [crossOriginNamed] },
        { video_generation: taskManifest },
        'https://provider.example/v1',
        [],
        [],
        ['media'],
        'bearer',
        { media: 'bearer' },
        connections
      ).errors.some(
        (error) =>
          error.task === 'video_generation' && error.code === 'cross_origin_consent_required'
      )
    ).toBe(true);
    expect(
      requiresCrossOriginConsent(
        { ...named, baseUrlOverride: 'https://other-media.example/v1' },
        taskManifest,
        'https://provider.example/v1',
        connections
      )
    ).toBe(true);
  });
});

describe('model id entry', () => {
  test('trims arbitrary ids and rejects exact duplicates without case folding', () => {
    expect(normalizeModelId('  vendor/model-latest  ')).toBe('vendor/model-latest');
    expect(isDuplicateModelId(' vendor/model-latest ', ['vendor/model-latest'])).toBe(true);
    expect(isDuplicateModelId('Vendor/model-latest', ['vendor/model-latest'])).toBe(false);
  });
});

/**
 * A TTS adapter that requires a provider voice (StepFun) fails closed when
 * `provider_params.voice` is missing, and the raw JSON textarea never hinted
 * that a voice was needed. The dedicated control edits the same JSON so the
 * two views can never disagree.
 */
describe('provider params voice', () => {
  test('reads the voice out of the raw params JSON, tolerating blank and invalid input', () => {
    expect(providerParamVoice('{"voice":"cixingnansheng"}')).toBe('cixingnansheng');
    expect(providerParamVoice('{\n  "voice": "  tianmeinvsheng  "\n}')).toBe('tianmeinvsheng');
    expect(providerParamVoice('')).toBe('');
    expect(providerParamVoice('   ')).toBe('');
    expect(providerParamVoice('{"speed":1.2}')).toBe('');
    expect(providerParamVoice('not json')).toBe('');
    // A non-string voice is not a usable id and must not be surfaced as one.
    expect(providerParamVoice('{"voice":42}')).toBe('');
  });

  test('writes the voice back into the JSON while preserving unrelated params', () => {
    const withSpeed = withProviderParamVoice('{"speed":1.25}', 'cixingnansheng');
    expect(JSON.parse(withSpeed)).toEqual({ speed: 1.25, voice: 'cixingnansheng' });

    // Round-trips through the reader.
    expect(providerParamVoice(withSpeed)).toBe('cixingnansheng');

    // Setting from empty produces a valid object, not a fragment.
    expect(JSON.parse(withProviderParamVoice('', 'boyinnansheng'))).toEqual({
      voice: 'boyinnansheng',
    });
  });

  test('clearing the voice removes the key and collapses an otherwise empty object to blank', () => {
    // Clearing must DELETE the key: an empty string would still fail the
    // adapter's non-empty check while looking configured in the UI.
    expect(JSON.parse(withProviderParamVoice('{"voice":"a","speed":1}', ''))).toEqual({ speed: 1 });
    expect(withProviderParamVoice('{"voice":"a"}', '')).toBe('');
    expect(withProviderParamVoice('{"voice":"a"}', '   ')).toBe('');
  });

  test('leaves malformed JSON untouched so a typo cannot silently discard the user text', () => {
    expect(withProviderParamVoice('not json', 'cixingnansheng')).toBe('not json');
  });
});

describe('openai.responses round chaining provider param', () => {
  test('reads only an explicit boolean true opt-in', () => {
    expect(providerParamChainRounds('{"chain_rounds":true}')).toBe(true);
    expect(providerParamChainRounds('{"chain_rounds":false}')).toBe(false);
    expect(providerParamChainRounds('{"chain_rounds":"true"}')).toBe(false);
    expect(providerParamChainRounds('{"temperature":0.2}')).toBe(false);
    expect(providerParamChainRounds('not json')).toBe(false);
  });

  test('writes true and preserves every unrelated provider param', () => {
    const updated = withProviderParamChainRounds(
      '{"temperature":0.2,"nested":{"keep":true}}',
      true
    );
    expect(JSON.parse(updated)).toEqual({
      temperature: 0.2,
      nested: { keep: true },
      chain_rounds: true,
    });
    expect(providerParamChainRounds(updated)).toBe(true);
  });

  test('disabled deletes the key and collapses an otherwise empty object', () => {
    expect(JSON.parse(withProviderParamChainRounds('{"chain_rounds":true,"temperature":0.2}', false))).toEqual({
      temperature: 0.2,
    });
    expect(withProviderParamChainRounds('{"chain_rounds":false}', false)).toBe('');
    expect(withProviderParamChainRounds('{"chain_rounds":true}', false)).toBe('');
  });

  test('leaves malformed input byte-identical', () => {
    const malformed = ' {\n  "chain_rounds": tru';
    expect(withProviderParamChainRounds(malformed, true)).toBe(malformed);
    expect(withProviderParamChainRounds(malformed, false)).toBe(malformed);
  });
});
