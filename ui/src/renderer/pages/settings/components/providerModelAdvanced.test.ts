/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import {
  applyProviderAutoConfiguration,
  applyProviderCompatibilityMode,
  buildProviderAutoConfigurationTargets,
} from './providerAutoConfiguration';
import {
  acknowledgeCatalogTaskConflict,
  addCapabilityTask,
  applyCatalogSuggestion,
  capabilityDraftFromResponse,
  capabilityInputsFromDefinition,
  capabilityValidationMessageKey,
  changeCapabilityProtocol,
  changeModelDefinitionId,
  createModelDefinitionDraft,
  describeValidationErrors,
  effectiveBaseUrl,
  emptyCapabilityDraft,
  getCatalogTaskConflict,
  isProtocolAuthSchemeAllowed,
  isDuplicateModelId,
  normalizeModelId,
  patchCapabilityDraft,
  providerParamChainRounds,
  providerParamReasoningEffort,
  providerParamVoice,
  reasoningEffortsForProtocol,
  protocolSupportsReasoningEffort,
  reconcileCapabilityRecommendations,
  removeCapabilityTask,
  resolveModelInputChange,
  requiresCrossOriginConsent,
  withProviderParamVoice,
  withProviderParamChainRounds,
  withProviderParamReasoningEffort,
  withCatalogTaskEvidence,
  validateModelDefinition,
  type ModelCapabilityDraft,
  type ModelDefinitionDraft,
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
  test('generic and specialized entries preserve explicit task intent without guessing chat', () => {
    expect(createModelDefinitionDraft()).toEqual({ model: '', capabilities: [] });
    for (const task of ['speech_recognition', 'speech_synthesis', 'image_generation', 'embedding'] as const) {
      const draft = createModelDefinitionDraft(task);
      expect(draft.capabilities).toEqual([{ ...emptyCapabilityDraft(task), routeSource: 'user' }]);
      const selected = applyCatalogSuggestion(draft, {
        model: 'known-text-model', tasks: ['chat'], traits: [], tasksSource: 'provider_declared',
      });
      expect(selected.capabilities).toEqual(draft.capabilities);
      expect(getCatalogTaskConflict(selected)).toMatchObject({
        configuredTasks: [task], declaredTasks: ['chat'], acknowledged: false,
      });
    }
  });

  test('inferred and missing task provenance never auto-confirm catalog purposes', () => {
    for (const tasksSource of [undefined, 'inferred'] as const) {
      for (const tasks of [['chat'], ['speech_recognition'], ['speech_synthesis']] as ModelTask[][]) {
        const suggestion = { model: 'name-only-model', tasks, traits: [], tasksSource };
        const generic = applyCatalogSuggestion(createModelDefinitionDraft(), suggestion);
        expect(generic.capabilities).toEqual([]);
        expect(validateModelDefinition(generic, {}, 'https://provider.example').errors).toEqual([
          { code: 'capability_required' },
        ]);
        const explicit = createModelDefinitionDraft('speech_recognition');
        expect(applyCatalogSuggestion(explicit, suggestion).capabilities).toEqual(explicit.capabilities);
        expect(getCatalogTaskConflict(applyCatalogSuggestion(explicit, suggestion))).toBeUndefined();
      }
    }
    for (const tasksSource of ['provider_declared', 'official_documentation'] as const) {
      expect(applyCatalogSuggestion(createModelDefinitionDraft(), {
        model: 'verified-asr', tasks: ['speech_recognition'], tasksSource, traits: [],
      }).capabilities.map((capability) => capability.task)).toEqual(['speech_recognition']);
    }
  });

  test('provider protocol presets and discovery never manufacture a purpose for an unknown ID', () => {
    const unknown = changeModelDefinitionId(createModelDefinitionDraft(), 'unknown-voice-model');
    const manifests = { chat: manifest('chat', 'openai.chat_text') };
    for (const mode of ['auto', 'openai', 'anthropic'] as const) {
      expect(applyProviderCompatibilityMode(unknown, mode, true).capabilities).toEqual([]);
    }
    expect(buildProviderAutoConfigurationTargets(unknown, manifests, 'bearer', false)).toEqual([]);
    expect(applyProviderAutoConfiguration(unknown, [{
      task: 'chat', protocol: 'openai.chat_text', authScheme: 'bearer', confidence: 'verified',
    }]).capabilities).toEqual([]);
  });

  test('advisory token metadata can enrich an explicitly chosen matching purpose without establishing it', () => {
    const suggestion = {
      model: 'provider-model', tasks: ['chat' as const], traits: ['vision_input' as const],
      tasksSource: 'inferred' as const, contextLimit: 128_000, outputLimit: 16_000,
    };
    expect(applyCatalogSuggestion(createModelDefinitionDraft(), suggestion).capabilities).toEqual([]);
    const chat = applyCatalogSuggestion(createModelDefinitionDraft('chat'), suggestion);
    expect(chat.capabilities[0]).toMatchObject({
      task: 'chat', routeSource: 'user', contextLimit: 128_000, outputLimit: 16_000,
    });
    const asr = createModelDefinitionDraft('speech_recognition');
    expect(applyCatalogSuggestion(asr, suggestion).capabilities).toEqual(asr.capabilities);
  });

  test('changing a model ID clears automatic purposes and stale evidence while retaining explicit routes', () => {
    const automatic = applyCatalogSuggestion(createModelDefinitionDraft(), {
      model: 'declared-chat', displayName: 'Declared model', tasks: ['chat'], traits: [], tasksSource: 'provider_declared',
    });
    expect(changeModelDefinitionId(automatic, 'manual-asr')).toEqual({ model: 'manual-asr', capabilities: [] });
    expect(changeModelDefinitionId(automatic, ' declared-chat ')).toEqual({ ...automatic, model: ' declared-chat ' });
    const tts = createModelDefinitionDraft('speech_synthesis');
    const conflicted = applyCatalogSuggestion(tts, {
      model: 'declared-chat', tasks: ['chat'], traits: [], tasksSource: 'provider_declared',
    });
    const updated = changeModelDefinitionId(acknowledgeCatalogTaskConflict(conflicted), 'manual-tts');
    expect(updated.capabilities).toEqual(tts.capabilities);
    expect(updated).not.toHaveProperty('catalogTaskConflict');
  });

  test('verified catalog conflicts require acknowledgement and permit manually confirmed incomplete catalogs', () => {
    const manifests = { speech_recognition: manifest('speech_recognition', 'openai.audio_transcriptions') };
    let configured = createModelDefinitionDraft('speech_recognition');
    configured.capabilities = reconcileCapabilityRecommendations(configured.capabilities, manifests);
    const selected = applyCatalogSuggestion(configured, {
      model: 'opaque-account-alias', tasks: ['chat'], traits: [], tasksSource: 'provider_declared',
    });
    expect(validateModelDefinition(selected, manifests, 'https://provider.example/v1').errors).toEqual([
      { code: 'catalog_task_conflict' },
    ]);
    const acknowledged = acknowledgeCatalogTaskConflict(selected);
    expect(validateModelDefinition(acknowledged, manifests, 'https://provider.example/v1').valid).toBe(true);
    expect(capabilityInputsFromDefinition(acknowledged)).toEqual([{
      task: 'speech_recognition', protocol: 'openai.audio_transcriptions', connection_role: 'default',
    }]);
    expect(capabilityInputsFromDefinition(acknowledged)![0]).not.toHaveProperty('catalogTaskConflict');

    const nextSelection = applyCatalogSuggestion(acknowledged, {
      model: 'another-chat-model', tasks: ['chat'], traits: [], tasksSource: 'official_documentation',
    });
    expect(getCatalogTaskConflict(nextSelection)?.acknowledged).toBe(false);
    const changedPurposes = { ...acknowledged, capabilities: addCapabilityTask(acknowledged.capabilities, 'speech_synthesis') };
    expect(getCatalogTaskConflict(changedPurposes)).toMatchObject({
      configuredTasks: ['speech_recognition', 'speech_synthesis'], acknowledged: false,
    });
    expect(getCatalogTaskConflict({ ...selected, model: 'changed-manual-id' })).toBeUndefined();
    expect(getCatalogTaskConflict({ ...selected, capabilities: [] })).toBeUndefined();
  });

  test('a registered text protocol cannot be saved as ASR or TTS', () => {
    for (const task of ['speech_recognition', 'speech_synthesis'] as const) {
      const wrongManifest = manifest('chat', 'openai.chat_text');
      const capability = { ...emptyCapabilityDraft(task), protocol: 'openai.chat_text' };
      expect(validateModelDefinition({ model: 'audio-model', capabilities: [capability] }, {
        [task]: wrongManifest,
      }, 'https://provider.example/v1').errors).toContainEqual({ task, code: 'protocol_task_mismatch' });
    }
  });

  test('manually typing a known model applies verified conflict evidence without changing configuration', () => {
    const task = 'speech_recognition' as const;
    const capability = patchCapabilityDraft(emptyCapabilityDraft(task), {
      protocol: 'openai.audio_transcriptions', outputLimit: 8_000, providerParamsJson: '{"temperature":0.2}',
    });
    const definition: ModelDefinitionDraft = {
      model: ' known-chat ', displayName: 'My recognizer', capabilities: [capability],
    };
    const suggestion = {
      model: 'known-chat', displayName: 'Official title', tasks: ['chat' as const], traits: ['vision_input' as const],
      tasksSource: 'provider_declared' as const, contextLimit: 100_000, outputLimit: 20_000,
    };
    const reconciled = withCatalogTaskEvidence(definition, suggestion);
    expect(getCatalogTaskConflict(reconciled)).toMatchObject({
      model: 'known-chat', configuredTasks: [task], declaredTasks: ['chat'], acknowledged: false,
    });
    expect(reconciled.model).toBe(definition.model);
    expect(reconciled.displayName).toBe('My recognizer');
    expect(reconciled.capabilities).toBe(definition.capabilities);
    expect(reconciled.capabilities[0]).toBe(capability);
    expect(validateModelDefinition(reconciled, {
      [task]: manifest(task, 'openai.audio_transcriptions'),
    }, 'https://provider.example/v1').errors).toContainEqual({ code: 'catalog_task_conflict' });

    const acknowledged = acknowledgeCatalogTaskConflict(reconciled);
    const typedAway = changeModelDefinitionId(acknowledged, 'known-cha');
    expect(withCatalogTaskEvidence(typedAway, suggestion)).toBe(typedAway);
    const typedBack = changeModelDefinitionId(typedAway, 'known-chat');
    expect(getCatalogTaskConflict(withCatalogTaskEvidence(typedBack, suggestion))?.acknowledged).toBe(false);
  });

  test('late catalog evidence adds a conflict without assigning purposes and ignores unknown or unrelated entries', () => {
    const definition = changeModelDefinitionId(createModelDefinitionDraft('speech_synthesis'), 'known-chat');
    const suggestion = {
      model: 'known-chat', tasks: ['chat' as const], traits: [], tasksSource: 'official_documentation' as const,
    };
    expect(withCatalogTaskEvidence(definition)).toBe(definition);
    expect(withCatalogTaskEvidence(definition, { ...suggestion, tasksSource: undefined })).toBe(definition);
    expect(withCatalogTaskEvidence(definition, { ...suggestion, tasksSource: 'inferred' })).toBe(definition);
    expect(withCatalogTaskEvidence(definition, { ...suggestion, tasks: [] })).toBe(definition);
    expect(withCatalogTaskEvidence(definition, { ...suggestion, model: 'other-model' })).toBe(definition);
    const late = withCatalogTaskEvidence(definition, suggestion);
    expect(late.capabilities).toBe(definition.capabilities);
    expect(getCatalogTaskConflict(late)?.acknowledged).toBe(false);
    expect(validateModelDefinition(late, {}, 'https://provider.example/v1').errors).toContainEqual({
      code: 'catalog_task_conflict',
    });

    const general = changeModelDefinitionId(createModelDefinitionDraft(), suggestion.model);
    expect(withCatalogTaskEvidence(general, suggestion)).toBe(general);
    expect(general.capabilities).toEqual([]);
  });

  test('unchanged catalog evidence is referentially stable and preserves an acknowledgement', () => {
    const definition = changeModelDefinitionId(createModelDefinitionDraft('speech_recognition'), 'known-model');
    const suggestion = {
      model: 'known-model', tasks: ['chat', 'embedding'] as ModelTask[], traits: [],
      tasksSource: 'provider_declared' as const,
    };
    const reconciled = withCatalogTaskEvidence(definition, suggestion);
    expect(withCatalogTaskEvidence(reconciled, suggestion)).toBe(reconciled);
    const acknowledged = acknowledgeCatalogTaskConflict(reconciled);
    expect(withCatalogTaskEvidence(acknowledged, suggestion)).toBe(acknowledged);
    expect(withCatalogTaskEvidence(acknowledged, {
      ...suggestion, model: ' known-model ', tasks: ['embedding', 'chat', 'embedding'],
    })).toBe(acknowledged);
    expect(getCatalogTaskConflict(acknowledged)?.acknowledged).toBe(true);
  });

  test('changed declared or configured task sets require fresh acknowledgement and resolved conflicts clear evidence', () => {
    const definition = changeModelDefinitionId(createModelDefinitionDraft('speech_recognition'), 'known-model');
    const suggestion = {
      model: 'known-model', tasks: ['chat' as const], traits: [], tasksSource: 'provider_declared' as const,
    };
    const acknowledged = acknowledgeCatalogTaskConflict(withCatalogTaskEvidence(definition, suggestion));
    const changedDeclaration = withCatalogTaskEvidence(acknowledged, { ...suggestion, tasks: ['embedding'] });
    expect(getCatalogTaskConflict(changedDeclaration)).toMatchObject({
      configuredTasks: ['speech_recognition'], declaredTasks: ['embedding'], acknowledged: false,
    });
    const changedConfiguration = withCatalogTaskEvidence({
      ...acknowledged, capabilities: addCapabilityTask(acknowledged.capabilities, 'speech_synthesis'),
    }, suggestion);
    expect(getCatalogTaskConflict(changedConfiguration)).toMatchObject({
      configuredTasks: ['speech_recognition', 'speech_synthesis'], acknowledged: false,
    });
    const resolved = withCatalogTaskEvidence(acknowledged, { ...suggestion, tasks: ['speech_recognition'] });
    expect(resolved).not.toHaveProperty('catalogTaskConflict');
    expect(resolved.capabilities).toBe(acknowledged.capabilities);
    expect(withCatalogTaskEvidence(resolved, { ...suggestion, tasks: ['speech_recognition'] })).toBe(resolved);
  });

  test('keeps free-text changes separate from the catalog onChange then onSelect event sequence', () => {
    let definition = { model: '', capabilities: [emptyCapabilityDraft('chat')] };

    const manualInput = resolveModelInputChange('vendor/custom-chat');
    if (manualInput !== undefined) definition = { ...definition, model: manualInput };
    expect(definition.model).toBe('vendor/custom-chat');

    const catalogInputChange = resolveModelInputChange('catalog/chat', { value: 'catalog/chat' });
    if (catalogInputChange !== undefined) definition = { ...definition, model: catalogInputChange };
    expect(definition.model).toBe('vendor/custom-chat');

    definition = applyCatalogSuggestion(
      definition,
      { model: 'catalog/chat', tasksSource: 'provider_declared' as const, tasks: ['chat', 'embedding'], traits: ['web_search'] }
    );
    expect(definition).toEqual({
      model: 'catalog/chat',
      capabilities: [{ ...emptyCapabilityDraft('chat'), traits: ['web_search'] }, emptyCapabilityDraft('embedding')],
    });
  });

  test('taskless and future catalog entries require an explicit purpose', () => {
    for (const capabilities of [[], [emptyCapabilityDraft('chat')]]) {
      const applied = applyCatalogSuggestion({ model: '', capabilities }, {
        model: 'vendor/future-model-2099', tasksSource: 'provider_declared' as const, tasks: [], traits: [],
      });
      expect(applied).toEqual({ model: 'vendor/future-model-2099', capabilities: [] });
      const manifests = { chat: manifest('chat', 'openai.chat_text') };
      const recommended = reconcileCapabilityRecommendations(applied.capabilities, manifests);
      expect(validateModelDefinition({ ...applied, capabilities: recommended }, manifests, 'https://provider.example/v1').errors).toEqual([
        { code: 'capability_required' },
      ]);
    }
  });

  test('adopting a catalog model preserves every other configured task', () => {
    const oldChat: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('chat'),
      routeSource: 'user',
      traits: ['web_search'],
      protocol: 'old.chat',
      endpoint: '/old/chat',
      providerParamsJson: '{"old":true}',
    };
    const applied = applyCatalogSuggestion(
      { model: 'old/model', capabilities: [oldChat] },
      {
        model: 'catalog/model',
        tasksSource: 'provider_declared' as const, tasks: ['speech_synthesis', 'realtime_conversation'],
        traits: [
          'web_search',
          'vision_input',
          'audio_input',
        ],
      }
    );

    expect(applied.model).toBe('catalog/model');
    // Catalog selection never converts or extends authored call routes.
    expect(applied.capabilities).toEqual([oldChat]);
    expect(applied.capabilities[0]).toBe(oldChat);
  });

  test('adopts each explicitly declared catalog route from an untouched automatic chat draft', () => {
    const applied = applyCatalogSuggestion(
      {
        model: '',
        capabilities: [{ ...emptyCapabilityDraft('chat'), transportSource: 'recommendation', protocol: 'openai.chat_text' }],
      },
      {
        model: 'doubao-seedream-5-0-260128',
        tasksSource: 'provider_declared' as const, tasks: ['image_generation', 'image_edit', 'image_generation'],
        traits: [],
      }
    );

    expect(applied.capabilities).toEqual([
      emptyCapabilityDraft('image_generation'),
      emptyCapabilityDraft('image_edit'),
    ]);

    const opaque = applyCatalogSuggestion(
      { model: '', capabilities: [emptyCapabilityDraft('chat')] },
      { model: 'ep-opaque', tasksSource: 'provider_declared' as const, tasks: ['image_generation'], traits: [] }
    );
    expect(opaque.capabilities).toEqual([emptyCapabilityDraft('image_generation')]);
  });

  test('automatic catalog routes switch Chat to image/edit and back without retaining old model metadata', () => {
    const chatManifest = manifest('chat', 'openai.chat_text', 'https://automatic.example/v1');
    chatManifest.recommendation!.base_url_override_required = true;
    let definition: ModelDefinitionDraft = { model: '', capabilities: [emptyCapabilityDraft('chat')] };
    definition = applyCatalogSuggestion(definition, {
      model: 'chat-a', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: ['vision_input'],
      contextLimit: 128_000, outputLimit: 16_000, contextLimitKind: 'input_only',
    });
    definition.capabilities = reconcileCapabilityRecommendations(definition.capabilities, { chat: chatManifest });
    expect(definition.capabilities[0]).toMatchObject({
      routeSource: 'automatic', baseUrlOverride: 'https://automatic.example/v1',
      contextLimit: 128_000, outputLimit: 16_000, traits: ['vision_input'],
    });

    const changedChat = applyCatalogSuggestion(definition, {
      model: 'chat-a2', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: ['audio_input'], contextLimit: 32_000,
    });
    expect(changedChat.capabilities[0]).toMatchObject({
      protocol: 'openai.chat_text', baseUrlOverride: 'https://automatic.example/v1',
      contextLimit: 32_000, outputLimit: undefined, traits: ['audio_input'], providerParamsJson: '',
    });

    definition = applyCatalogSuggestion(definition, {
      model: 'image-b', tasksSource: 'provider_declared' as const, tasks: ['image_generation', 'image_edit'], traits: [],
      contextLimit: 64_000, outputLimit: 4_000, contextLimitKind: 'combined',
    });
    expect(definition.capabilities.map((route) => route.task)).toEqual(['image_generation', 'image_edit']);
    expect(definition.capabilities.every((route) => route.routeSource === 'automatic')).toBe(true);

    definition = applyCatalogSuggestion(definition, {
      model: 'chat-c', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: [], contextLimit: 48_000,
    });
    expect(definition.capabilities).toEqual([{ ...emptyCapabilityDraft('chat'), contextLimit: 48_000 }]);
    definition = applyCatalogSuggestion(definition, {
      model: 'image-d', tasksSource: 'provider_declared' as const, tasks: ['image_generation'], traits: [], contextLimit: 64_000,
    });
    definition = applyCatalogSuggestion(definition, { model: 'future-unknown', tasksSource: 'provider_declared' as const, tasks: [], traits: [] });
    expect(definition.capabilities).toEqual([]);
    expect(capabilityInputsFromDefinition(definition)).toEqual([]);
  });

  test('manual limits, protocol choices, and explicitly added routes preserve existing interfaces on catalog changes', () => {
    const initial = applyCatalogSuggestion({ model: '', capabilities: [emptyCapabilityDraft('chat')] }, {
      model: 'chat-a', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: ['vision_input'], contextLimit: 128_000, outputLimit: 16_000,
    });
    const image = { model: 'image-b', tasksSource: 'provider_declared' as const, tasks: ['image_generation' as const], traits: [] };
    for (const patch of [{ contextLimit: 96_000 }, { contextLimit: undefined }, { outputLimit: 2_000 }, { compactionThresholdPct: 60 }]) {
      const authored = patchCapabilityDraft(initial.capabilities[0], patch);
      expect(authored.routeSource).toBe('user');
      expect(applyCatalogSuggestion({ ...initial, capabilities: [authored] }, image).capabilities).toEqual([authored]);
    }
    const confirmed = changeCapabilityProtocol(initial.capabilities[0], 'openai.chat_text');
    expect(confirmed.routeSource).toBe('user');
    expect(applyCatalogSuggestion({ ...initial, capabilities: [confirmed] }, image).capabilities).toEqual([confirmed]);
    const manuallyExtended = addCapabilityTask(initial.capabilities, 'speech_synthesis');
    expect(manuallyExtended[1].routeSource).toBe('user');
    expect(applyCatalogSuggestion({ ...initial, capabilities: manuallyExtended }, image).capabilities).toEqual(manuallyExtended);
    const persisted = capabilityDraftFromResponse({ task: 'chat', protocol: 'openai.chat_text', connection_role: 'default' });
    expect(persisted.routeSource).toBe('persisted');
    expect(applyCatalogSuggestion({ ...initial, capabilities: [persisted] }, image).capabilities).toEqual([persisted]);
  });

  test('provider-wide compatibility presets keep untouched catalog routes automatic', () => {
    const initial = applyProviderCompatibilityMode({ model: '', capabilities: [emptyCapabilityDraft('chat')] }, 'openai', true);
    expect(initial.capabilities[0]).toMatchObject({ routeSource: 'automatic', transportSource: 'user', protocol: 'openai.chat_text' });
    const changedChat = applyCatalogSuggestion(initial, { model: 'chat-a', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: [] });
    expect(changedChat.capabilities[0].protocol).toBe('openai.chat_text');
    const image = applyProviderCompatibilityMode(applyCatalogSuggestion(changedChat, {
      model: 'image-b', tasksSource: 'provider_declared' as const, tasks: ['image_generation', 'image_edit'], traits: [],
    }), 'openai');
    expect(image.capabilities.map((route) => [route.task, route.protocol])).toEqual([
      ['image_generation', 'openai.images'], ['image_edit', 'openai.images'],
    ]);
    const chat = applyProviderCompatibilityMode(applyCatalogSuggestion(image, { model: 'future-chat', tasksSource: 'provider_declared' as const, tasks: [], traits: [] }), 'openai');
    expect(chat.capabilities).toEqual([]);
  });

  test('an explicit initial route survives selecting a taskless or differently classified model', () => {
    for (const task of ['image_generation', 'embedding'] as const) {
      const route: ModelCapabilityDraft = { ...emptyCapabilityDraft(task), routeSource: 'user' };
      for (const tasks of [[], ['chat' as const]]) {
        const selected = applyCatalogSuggestion({ model: 'deep-linked-model', capabilities: [route] }, {
          model: 'future-model', tasks, traits: [],
        });
        expect(selected.capabilities).toEqual([route]);
        expect(selected.capabilities[0]).toBe(route);
      }
    }
  });

  test('adopting a catalog model keeps the chosen task transport and only refreshes its traits', () => {
    const configuredChat: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('chat'),
      routeSource: 'user',
      traits: ['audio_input'],
      protocol: 'openai.chat_text',
      endpoint: '/chat/completions',
      providerParamsJson: '{"temperature":0.2}',
    };

    expect(
      applyCatalogSuggestion(
        { model: 'old/model', capabilities: [configuredChat] },
        { model: 'catalog/chat', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: ['web_search', 'vision_input'] }
      )
    ).toEqual({
      model: 'catalog/chat',
      capabilities: [{ ...configuredChat, traits: ['vision_input', 'web_search'] }],
    });
  });

  test('prefills a provider-declared context window without overriding the user', () => {
    // Only an explicit provider declaration supplies a numeric window.
    expect(
      applyCatalogSuggestion(
        { model: '', capabilities: [] },
        { model: 'catalog/chat', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: [], contextLimit: 32_000 }
      ).capabilities[0]?.contextLimit
    ).toBe(32_000);

    // An explicit user value wins: correcting the provider is the point.
    expect(
      applyCatalogSuggestion(
        {
          model: '',
          capabilities: [patchCapabilityDraft(emptyCapabilityDraft('chat'), { contextLimit: 8_000 })],
        },
        { model: 'catalog/chat', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: [], contextLimit: 32_000 }
      ).capabilities[0]?.contextLimit
    ).toBe(8_000);

    // A provider that declares nothing must not manufacture a window.
    expect(
      applyCatalogSuggestion(
        { model: '', capabilities: [] },
        { model: 'catalog/chat', tasksSource: 'provider_declared' as const, tasks: ['chat'], traits: [] }
      ).capabilities[0]?.contextLimit
    ).toBeUndefined();
  });

  test('catalog limits are advisory, including an explicit provider-default choice', () => {
    const suggestion = { model: 'catalog/model', tasksSource: 'provider_declared' as const, tasks: ['chat' as const], traits: ['vision_input' as const],
      contextLimit: 1_000_000, outputLimit: 100_000 };
    const fresh = applyCatalogSuggestion({model:'',capabilities:[emptyCapabilityDraft('chat')]},suggestion);
    expect(fresh.capabilities[0]).toMatchObject({contextLimit:1_000_000,outputLimit:100_000,traits:['vision_input']});
    const explicit = patchCapabilityDraft(emptyCapabilityDraft('chat'), {
      contextLimit: undefined, outputLimit: undefined,
      providerParamsJson:'{"reasoning_effort":"high"}',
    });
    const preserved = applyCatalogSuggestion({model:'old',capabilities:[explicit]},suggestion);
    expect(preserved.capabilities[0]).toMatchObject({contextLimit:undefined,outputLimit:undefined,
      providerParamsJson:explicit.providerParamsJson});
    const persisted = capabilityDraftFromResponse({task:'chat',traits:[],protocol:'openai.chat_text',connection_role:'default'});
    const unchanged = applyCatalogSuggestion({model:'old',capabilities:[persisted]},suggestion);
    expect(unchanged.capabilities[0]).toMatchObject({contextLimit:undefined,outputLimit:undefined,traits:[]});
    const numeric = patchCapabilityDraft(explicit,{contextLimit:2_000_000,outputLimit:200_000});
    const kept = applyCatalogSuggestion({model:'old',capabilities:[numeric]},suggestion);
    expect(kept.capabilities[0]).toMatchObject({contextLimit:2_000_000,outputLimit:200_000});
    expect(capabilityInputsFromDefinition(preserved)![0]).not.toHaveProperty('outputLimitSource');
  });

  test('token validation accepts the wire integer range without imposing a universal model ceiling', () => {
    const manifests = {chat:manifest('chat','openai.chat_text')};
    const validate = (contextLimit?:number,outputLimit?:number) => validateModelDefinition({model:'m',capabilities:[
      {...emptyCapabilityDraft('chat'),protocol:'openai.chat_text',contextLimit,outputLimit}
    ]},manifests,'https://provider.example');
    for (const value of [undefined,1_000_000,100_000_000,0xffff_ffff]) expect(validate(value,value).valid).toBe(true);
    for (const value of [0,-1,1.5,Number.NaN,Number.POSITIVE_INFINITY,0x1_0000_0000]) {
      expect(validate(value,value).errors.some(error=>error.code==='invalid_token_limit')).toBe(true);
    }
  });

  test('catalog context semantics are imported only with a new declared window, never over user modes', () => {
    const suggestion={model:'declared',tasksSource: 'provider_declared' as const, tasks:['chat' as const],traits:[],contextLimit:1_000_000,
      outputLimit:100_000,contextLimitKind:'input_only' as const};
    const applied=applyCatalogSuggestion({model:'',capabilities:[emptyCapabilityDraft('chat')]},suggestion);
    expect(JSON.parse(applied.capabilities[0].providerParamsJson)).toEqual({_nomifun_context_limit_kind:'input_only'});
    expect(capabilityInputsFromDefinition(applied)![0].provider_params).toEqual({_nomifun_context_limit_kind:'input_only'});
    for(const kind of [undefined,'combined' as const]) {
      const adopted=applyCatalogSuggestion({model:'',capabilities:[]},{...suggestion,contextLimitKind:kind});
      expect(adopted.capabilities[0].providerParamsJson).toBe(kind===undefined?'':JSON.stringify({_nomifun_context_limit_kind:'combined'},null,2));
    }
    for(const capability of [
      patchCapabilityDraft(emptyCapabilityDraft('chat'),{contextLimit:undefined}),
      patchCapabilityDraft(emptyCapabilityDraft('chat'),{contextLimit:200_000}),
      capabilityDraftFromResponse({task:'chat',traits:[],protocol:'openai.chat_text',connection_role:'default'}),
      patchCapabilityDraft(emptyCapabilityDraft('chat'),{providerParamsJson:'{"_nomifun_context_limit_kind":"combined","temperature":0.2}'}),
      patchCapabilityDraft(emptyCapabilityDraft('chat'),{providerParamsJson:'{"temperature":'}),
    ]) {
      const updated=applyCatalogSuggestion({model:'old',capabilities:[capability]},suggestion);
      expect(updated.capabilities[0].providerParamsJson).toBe(capability.providerParamsJson);
    }
  });

  test('a generic manually entered ID requires purpose before reaching a saveable draft', () => {
    const manifests = { chat: manifest('chat', 'openai.chat_text') };

    let definition = createModelDefinitionDraft();
    expect(
      validateModelDefinition(definition, manifests, 'https://api.stepfun.com/v1').errors.map(
        (error) => error.code
      )
    ).toEqual(['model_required', 'capability_required']);

    definition = changeModelDefinitionId(definition, 'step-3.7-flash');
    expect(validateModelDefinition(definition, manifests, 'https://api.stepfun.com/v1').errors).toEqual([
      { code: 'capability_required' },
    ]);
    definition = { ...definition, capabilities: addCapabilityTask(definition.capabilities, 'chat') };

    // Only after the explicit purpose selection may transport recommendations apply.
    definition = {
      ...definition,
      capabilities: reconcileCapabilityRecommendations(definition.capabilities, manifests),
    };
    expect(definition.capabilities[0]?.protocol).toBe('openai.chat_text');

    const result = validateModelDefinition(definition, manifests, 'https://api.stepfun.com/v1');
    expect(result.errors).toEqual([]);
    expect(result.valid).toBe(true);
  });

  test('a gateway that ships no recommended protocol says so instead of just failing', () => {
    // Reported: with a `new-api` provider the top of the form is complete
    // (base URL, key, model) but the model cannot be saved and nothing says
    // why. new-api deliberately gets no preset protocol recommendation
    // (routes_table.rs: ("custom" | "new-api", _) => None), so a blank protocol
    // is the real and only blocker — it just has to be stated.
    const manifests = { chat: manifest('chat', 'openai.chat_text') };
    const definition: ModelDefinitionDraft = {
      model: 'gpt-4o',
      // No recommendation was applied, so the protocol stays blank.
      capabilities: [emptyCapabilityDraft('chat')],
    };

    const result = validateModelDefinition(definition, manifests, 'https://gateway.example/v1');
    expect(result.errors).toEqual([{ task: 'chat', code: 'protocol_required' }]);

    const described = describeValidationErrors(
      result.errors,
      (key, fallback) => (key === 'settings.capabilityError.protocol_required' ? '请选择调用协议' : fallback)
    );
    expect(described).toBe('chat · 请选择调用协议');
  });

  test('validation messages are named per code and deduplicated', () => {
    expect(capabilityValidationMessageKey('protocol_required')).toBe(
      'settings.capabilityError.protocol_required'
    );
    // The same code on two tasks stays two lines; an identical model-level code
    // collapses to one.
    expect(
      describeValidationErrors(
        [
          { code: 'model_required' },
          { code: 'model_required' },
          { task: 'chat', code: 'base_url_required' },
          { task: 'embedding', code: 'base_url_required' },
        ],
        (_key, fallback) => fallback
      )
    ).toBe('model_required · chat · base_url_required · embedding · base_url_required');
  });

  test('does not touch traits when the selected task is absent from the entry', () => {
    const oldSpeech: ModelCapabilityDraft = {
      ...emptyCapabilityDraft('speech_synthesis'),
      routeSource: 'user',
      traits: [],
      protocol: 'old.speech',
      endpoint: '/old/speech',
    };

    expect(
      applyCatalogSuggestion(
        { model: 'old/model', capabilities: [oldSpeech] },
        { model: 'catalog/unknown', tasksSource: 'provider_declared' as const, tasks: [], traits: ['audio_input'] }
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

    expect(withSpeech).toEqual([chat, { ...emptyCapabilityDraft('speech_synthesis'), routeSource: 'user' }]);
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
      traits: [],
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
      routeSource: 'user',
      transportSource: 'user',
    });
    const changed = changeCapabilityProtocol(current, 'openai.audio_speech', taskManifest);
    expect(changed).toEqual({
      ...current,
      routeSource: 'user',
      transportSource: 'user',
      protocol: 'openai.audio_speech',
      connectionRole: 'default',
      baseUrlOverride: '',
      endpoint: '',
      pollEndpoint: '',
      contentEndpoint: '',
      realtimeEndpoint: '',
      allowCrossOriginCredentials: false,
      providerParamsJson: '{"voice":"alloy"}',
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

  test('round-trips a Chat compaction threshold through the capability save', () => {
    const input = capabilityInputsFromDefinition({
      model: 'chat-model',
      capabilities: [{
        ...emptyCapabilityDraft('chat'),
        protocol: 'openai.chat_text',
        contextLimit: 64_000,
        compactionThresholdPct: 60,
      }],
    });
    expect(input?.[0]).toMatchObject({
      context_limit: 64_000,
      compaction_threshold_pct: 60,
    });
    const restored = capabilityDraftFromResponse({
      task: 'chat',
      protocol: 'openai.chat_text',
      connection_role: 'default',
      context_limit: 64_000,
      compaction_threshold_pct: 60,
    });
    expect(restored.contextLimit).toBe(64_000);
    expect(restored.compactionThresholdPct).toBe(60);
  });

  test('round-trips one persisted capability into the typed editor draft', () => {
    expect(
      capabilityDraftFromResponse({
        task: 'speech_synthesis',
        traits: [],
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
      traits: [],
      routeSource: 'persisted',
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

describe('model reasoning effort', () => {
  test('reads, writes, clears, and preserves unrelated provider params', () => {
    expect(providerParamReasoningEffort('{"reasoning_effort":"medium"}')).toBe('medium');
    expect(providerParamReasoningEffort('{"reasoning_effort":"max"}')).toBe('max');
    expect(providerParamReasoningEffort('{"reasoning_effort":"ultra"}')).toBe('ultra');
    expect(providerParamReasoningEffort('{"reasoning_effort":"extreme"}')).toBeUndefined();
    expect(providerParamReasoningEffort('not json')).toBeUndefined();

    const configured = withProviderParamReasoningEffort('{"temperature":0.2}', 'high');
    expect(JSON.parse(configured)).toEqual({ temperature: 0.2, reasoning_effort: 'high' });
    expect(providerParamReasoningEffort(configured)).toBe('high');
    expect(JSON.parse(withProviderParamReasoningEffort(configured, undefined))).toEqual({
      temperature: 0.2,
    });
    expect(withProviderParamReasoningEffort('{"reasoning_effort":"low"}', undefined)).toBe('');
    expect(withProviderParamReasoningEffort('not json', 'low')).toBe('not json');
  });

  test('recognizes only protocols with a normalized reasoning-effort mapping', () => {
    for (const protocol of ['openai.chat_text', 'openai.responses', 'gemini.generate_text']) {
      expect(protocolSupportsReasoningEffort(protocol)).toBe(true);
    }
    for (const protocol of ['anthropic.messages', 'bedrock.anthropic_messages', '']) {
      expect(protocolSupportsReasoningEffort(protocol)).toBe(false);
    }
    expect(reasoningEffortsForProtocol('openai.responses')).toEqual([
      'none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra',
    ]);
    expect(reasoningEffortsForProtocol('gemini.generate_text')).toEqual([
      'minimal', 'low', 'medium', 'high',
    ]);
  });

  test('explicit none/minimal survive JSON separately from provider defaults and Gemini rejects none', () => {
    for(const effort of ['none','minimal'] as const) {
      const params=withProviderParamReasoningEffort('{"temperature":0.2}',effort);
      expect(providerParamReasoningEffort(params)).toBe(effort);
      expect(JSON.parse(withProviderParamReasoningEffort(params,undefined))).toEqual({temperature:0.2});
      const capability={...emptyCapabilityDraft('chat'),protocol:'openai.chat_text',providerParamsJson:params};
      expect(validateModelDefinition({model:'m',capabilities:[capability]},
        {chat:manifest('chat','openai.chat_text')},'https://provider.example').valid).toBe(true);
      const google={...capability,protocol:'gemini.generate_text'};
      const validation=validateModelDefinition({model:'m',capabilities:[google]},
        {chat:manifest('chat','gemini.generate_text')},'https://provider.example');
      expect(validation.valid).toBe(effort==='minimal');
    }
  });

  test('validation rejects malformed or unsupported authored reasoning levels', () => {
    const chatManifest = manifest('chat', 'openai.chat_text');
    const base = {
      ...emptyCapabilityDraft('chat'),
      protocol: 'openai.chat_text',
      transportSource: 'user' as const,
    };
    expect(validateModelDefinition(
      { model: 'reasoner', capabilities: [{ ...base, providerParamsJson: '{"reasoning_effort":"high"}' }] },
      { chat: chatManifest },
      'https://api.stepfun.com/v1'
    ).valid).toBe(true);
    expect(validateModelDefinition(
      { model: 'reasoner', capabilities: [{ ...base, providerParamsJson: '{"reasoning_effort":"ultra"}' }] },
      { chat: chatManifest },
      'https://api.stepfun.com/v1'
    ).valid).toBe(true);
    expect(validateModelDefinition(
      { model: 'reasoner', capabilities: [{ ...base, providerParamsJson: '{"reasoning_effort":"extreme"}' }] },
      { chat: chatManifest },
      'https://api.stepfun.com/v1'
    ).errors).toContainEqual({ task: 'chat', code: 'invalid_provider_params' });
    expect(validateModelDefinition(
      { model: 'reasoner', capabilities: [{
        ...base,
        protocol: 'gemini.generate_text',
        providerParamsJson: '{"reasoning_effort":"max"}',
      }] },
      { chat: manifest('chat', 'gemini.generate_text') },
      'https://api.stepfun.com/v1'
    ).errors).toContainEqual({ task: 'chat', code: 'invalid_provider_params' });
    expect(validateModelDefinition(
      { model: 'reasoner', capabilities: [{
        ...base,
        protocol: 'anthropic.messages',
        providerParamsJson: '{"reasoning_effort":"low"}',
      }] },
      { chat: manifest('chat', 'anthropic.messages') },
      'https://api.stepfun.com/v1'
    ).errors).toContainEqual({ task: 'chat', code: 'invalid_provider_params' });
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
