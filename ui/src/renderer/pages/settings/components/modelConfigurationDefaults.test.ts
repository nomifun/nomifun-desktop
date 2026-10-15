import { describe, expect, test } from 'bun:test';
import { aliasProtocolManifest } from '../../../../../test/fixtures/modelAliasEditor';
import {
  applyCatalogSuggestion, capabilityDraftFromResponse, capabilityInputsFromDefinition,
  changeModelDefinitionId, createModelDefinitionDraft, emptyCapabilityDraft, patchCapabilityDraft,
  reasoningEffortsForCapability, reconcileCapabilityRecommendations, type ModelProtocolManifest,
} from './providerModelAdvanced';

const manifest = (model = 'documented/model', protocol = 'openai.responses'): ModelProtocolManifest => ({
  ...aliasProtocolManifest,
  recommendation: { ...aliasProtocolManifest.recommendation!, protocol_id: protocol },
  protocols: [{ ...aliasProtocolManifest.protocols[0], protocol_id: protocol }],
  model_defaults: {
    model, protocol_id: protocol, context_limit: 1_000_000, context_limit_kind: 'combined',
    output_limit: 128_000, provider_params: { reasoning_effort: 'high' },
    reasoning_efforts: ['low', 'high'], sources: ['https://vendor.example/docs/model'], checked_at: '2026-10-15',
  },
});

describe('exact documented model configuration defaults', () => {
  test('fills a fresh explicit purpose and serializes only canonical values, without mutating the input', () => {
    const original = emptyCapabilityDraft('chat');
    const [configured] = reconcileCapabilityRecommendations([original], { chat: manifest() }, 'documented/model');
    expect(original).toEqual(emptyCapabilityDraft('chat'));
    expect(configured).toMatchObject({ protocol: 'openai.responses', contextLimit: 1_000_000, outputLimit: 128_000 });
    expect(JSON.parse(configured.providerParamsJson)).toEqual({ reasoning_effort: 'high', _nomifun_context_limit_kind: 'combined' });
    const wire = capabilityInputsFromDefinition({ model: 'documented/model', capabilities: [configured] })![0];
    expect(wire).not.toHaveProperty('modelDefaults');
    expect(wire).not.toHaveProperty('providerParamsSource');
    expect(JSON.stringify(reconcileCapabilityRecommendations([configured], { chat: manifest() }, 'documented/model'))).toBe(JSON.stringify([configured]));
  });

  test('ignores a stale model response, including its transport recommendation', () => {
    const original = emptyCapabilityDraft('chat');
    expect(reconcileCapabilityRecommendations([original], { chat: manifest('previous/model') }, 'new/model')).toEqual([original]);
    const unknown = reconcileCapabilityRecommendations([original], { chat: { ...manifest(), model_defaults: undefined } }, 'unlisted/model');
    expect(unknown[0].contextLimit).toBeUndefined();
    expect(unknown[0].providerParamsJson).toBe('');
  });

  test('model ID changes remove only earlier auto values and retain explicit limits, JSON, and cleared defaults', () => {
    const [configured] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], { chat: manifest() }, 'documented/model');
    const manualLimit = patchCapabilityDraft(configured, { contextLimit: 64_000, outputLimit: undefined });
    const changed = changeModelDefinitionId({ model: 'documented/model', capabilities: [manualLimit] }, 'another/model');
    expect(changed.capabilities[0]).toMatchObject({ contextLimit: 64_000, outputLimit: undefined, providerParamsJson: '' });
    expect(changed.capabilities[0]).not.toHaveProperty('modelDefaults');
    const [refilled] = reconcileCapabilityRecommendations(changed.capabilities, { chat: manifest('another/model') }, 'another/model');
    expect(refilled.contextLimit).toBe(64_000);
    expect(refilled.outputLimit).toBeUndefined();
    const authored = patchCapabilityDraft(configured, { providerParamsJson: '{"reasoning_effort":"low","future":true}' });
    const authoredChange = changeModelDefinitionId({ model: 'documented/model', capabilities: [authored] }, 'another/model');
    expect(authoredChange.capabilities[0].providerParamsJson).toBe(authored.providerParamsJson);
    expect(authoredChange.capabilities[0].contextLimit).toBeUndefined();
    const cleared = patchCapabilityDraft(configured, { providerParamsJson: '' });
    expect(reconcileCapabilityRecommendations([cleared], { chat: manifest() }, 'documented/model')[0].providerParamsJson).toBe('');
  });

  test('preserves persisted and user-owned protocols and never converts a generic purpose-free form', () => {
    for (const transportSource of ['persisted', 'user'] as const) {
      const saved = { ...capabilityDraftFromResponse({ task: 'chat', protocol: 'openai.chat_text', connection_role: 'default',
        context_limit: 32_000, output_limit: 4_000, provider_params: { temperature: 0.4 } }), transportSource };
      expect(reconcileCapabilityRecommendations([saved], { chat: manifest() }, 'documented/model')[0]).toBe(saved);
    }
    expect(reconcileCapabilityRecommendations(createModelDefinitionDraft().capabilities, { chat: manifest() }, 'documented/model')).toEqual([]);
  });

  test('provider catalog windows win whether they arrive before or after the model documentation', () => {
    const suggestion = { model: 'documented/model', tasks: ['chat'] as const, traits: [], tasksSource: 'provider_declared' as const,
      contextLimit: 256_000, contextLimitKind: 'input_only' as const, outputLimit: 16_384 };
    const initial = { ...createModelDefinitionDraft('chat'), model: 'documented/model' };
    const catalogFirst = applyCatalogSuggestion(initial, { ...suggestion, tasks: ['chat'] });
    const [configured] = reconcileCapabilityRecommendations(initial.capabilities, { chat: manifest() }, initial.model);
    const docsFirst = applyCatalogSuggestion({ ...initial, capabilities: [configured] }, { ...suggestion, tasks: ['chat'] });
    for (const definition of [catalogFirst, docsFirst]) {
      const [result] = reconcileCapabilityRecommendations(definition.capabilities, { chat: manifest() }, definition.model);
      expect(result.contextLimit).toBe(256_000);
      expect(result.outputLimit).toBe(16_384);
      expect(JSON.parse(result.providerParamsJson)._nomifun_context_limit_kind).toBe('input_only');
    }
  });

  test('uses exact model/protocol effort evidence and distinguishes documented none from unknown support', () => {
    const capability = { ...emptyCapabilityDraft('chat'), protocol: 'openai.responses' };
    expect(reasoningEffortsForCapability(capability, manifest(), 'documented/model')).toEqual(['low', 'high']);
    const none = manifest(); none.model_defaults!.reasoning_efforts = [];
    expect(reasoningEffortsForCapability(capability, none, 'documented/model')).toEqual([]);
    const unknown = manifest(); delete unknown.model_defaults!.reasoning_efforts;
    expect(reasoningEffortsForCapability(capability, unknown, 'documented/model')).toContain('xhigh');
    expect(reasoningEffortsForCapability(capability, none, 'different/model')).toContain('xhigh');
    expect(reasoningEffortsForCapability({ ...capability, protocol: 'openai.chat_text' }, none, 'documented/model')).toContain('high');
  });

  test('reselecting the same catalog ID does not discard unrelated auto values after a manual JSON edit', () => {
    const [configured] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], { chat: manifest() }, 'documented/model');
    const authored = patchCapabilityDraft(configured, { providerParamsJson: '{"temperature":0.5}' });
    const current = { model: 'documented/model', capabilities: [authored] };
    const selected = applyCatalogSuggestion(current, { model: current.model, tasks: ['chat'], traits: [] });
    expect(selected.capabilities[0].contextLimit).toBe(1_000_000);
    expect(selected.capabilities[0].outputLimit).toBe(128_000);
    expect(selected.capabilities[0].providerParamsJson).toBe(authored.providerParamsJson);
  });

  test('structured reasoning edits preserve the chosen level while retiring automatic context semantics on another model', () => {
    const [configured] = reconcileCapabilityRecommendations([emptyCapabilityDraft('chat')], { chat: manifest() }, 'documented/model');
    const manual = patchCapabilityDraft(configured, {
      providerParamsEditedKeys: ['reasoning_effort'],
      providerParamsJson: '{"reasoning_effort":"high","_nomifun_context_limit_kind":"combined"}',
    });
    const changed = changeModelDefinitionId({ model: 'documented/model', capabilities: [manual] }, 'another/model');
    expect(JSON.parse(changed.capabilities[0].providerParamsJson)).toEqual({ reasoning_effort: 'high' });
    expect(changed.capabilities[0].contextLimit).toBeUndefined();
    expect(capabilityInputsFromDefinition(changed)![0]).not.toHaveProperty('providerParamsEditedKeys');
  });
});
