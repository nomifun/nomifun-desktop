import { describe, expect, test } from 'bun:test';
import { aliasProtocolManifest, aliasProviderBaseUrl } from '../../../../../test/fixtures/modelAliasEditor';
import { restoreCapabilityProtocolParams, suspendCapabilityTransitionIssues } from './chatProtocolDraftTransition';
import { applyProviderCompatibilityMode } from './providerAutoConfiguration';
import { applyDetectedChatProtocol, chatProtocolDetectionSnapshot } from './modelProtocolDetection';
import { capabilityInputsFromDefinition, changeCapabilityProtocol, emptyCapabilityDraft, patchCapabilityDraft,
  validateModelDefinition, type ModelCapabilityDraft, type ModelDefinitionDraft } from './providerModelAdvanced';

const chat = 'openai.chat_text'; const responses = 'openai.responses';
const cap = (params: Record<string, unknown>, protocol = chat): ModelCapabilityDraft => ({
  ...emptyCapabilityDraft('chat'), protocol, transportSource: 'persisted', providerParamsJson: JSON.stringify(params), outputLimit: 16_384,
});
const params = (capability: ModelCapabilityDraft): Record<string, unknown> => JSON.parse(capability.providerParamsJson || '{}');
const definition = (capability: ModelCapabilityDraft): ModelDefinitionDraft => ({ model: 'arbitrary/vendor-model', capabilities: [capability] });
const manifests = { chat: { ...aliasProtocolManifest, protocols: [aliasProtocolManifest.protocols[0],
  { ...aliasProtocolManifest.protocols[0], protocol_id: responses }] } };
const valid = (capability: ModelCapabilityDraft) => validateModelDefinition(definition(capability), manifests, aliasProviderBaseUrl).valid;

describe('protocol parameter draft transitions', () => {
  test('maps semantic settings and roundtrips local controls without overwriting newly authored settings', () => {
    const original = cap({ stream_options: null, max_tokens_field: 'max_completion_tokens', reasoning_effort: 'low', verbosity: 'low',
      response_format: { type: 'json_schema', json_schema: { name: 'answer', schema: { type: 'object' }, strict: true } }, thinking: { type: 'enabled' }, vendor: 1 });
    const converted = changeCapabilityProtocol(original, responses);
    expect(params(converted)).toEqual({ reasoning_effort: 'low', text: { verbosity: 'low',
      format: { type: 'json_schema', name: 'answer', schema: { type: 'object' }, strict: true } }, thinking: { type: 'enabled' }, vendor: 1 });
    expect(valid(converted)).toBe(true);
    const edited = patchCapabilityDraft(converted, { providerParamsJson: JSON.stringify({ ...params(converted), reasoning_effort: 'high',
      text: { ...(params(converted).text as object), verbosity: 'high' } }) });
    const returned = changeCapabilityProtocol(edited, chat);
    expect(params(returned)).toMatchObject({ stream_options: null, max_tokens_field: 'max_completion_tokens', reasoning_effort: 'high', verbosity: 'high',
      response_format: { type: 'json_schema', json_schema: { name: 'answer' } } });
    const wire = capabilityInputsFromDefinition(definition(returned))![0];
    expect(wire).not.toHaveProperty('protocolParameterStash');
    expect(wire).not.toHaveProperty('protocolParameterTransition');
    expect(JSON.stringify(wire)).not.toContain('restoredKeys');
  });

  test('unresolved semantic choices survive unrelated edits, same-protocol reselection and unrelated cache restore', () => {
    const switched = changeCapabilityProtocol(cap({ reasoning: { summary: 'auto' } }, responses), chat);
    switched.protocolParameterStash = { [chat]: { stream_options: null } };
    expect(valid(switched)).toBe(false);
    const edited = patchCapabilityDraft(switched, { providerParamsJson: JSON.stringify({ ...params(switched), temperature: 0.3 }) });
    expect(edited.protocolParameterTransition?.issues).toEqual([{ parameter: 'reasoning', kind: 'unsupported' }]);
    const reselected = changeCapabilityProtocol(edited, chat);
    expect(valid(reselected)).toBe(false);
    const restored = restoreCapabilityProtocolParams(reselected);
    expect(params(restored)).toEqual({ reasoning: { summary: 'auto' }, temperature: 0.3, stream_options: null });
    expect(valid(restored)).toBe(false);
    expect(restored.protocolParameterTransition?.source).toBe(responses);
  });

  test('suspends an unknown text remainder in one click and deeply restores it beside newly mapped format', () => {
    const source = cap({ text: { format: { type: 'json_object' }, vendor_style: { compact: true } }, thinking: { budget: 9 } }, responses);
    const switched = changeCapabilityProtocol(source, chat);
    expect(switched.protocolParameterTransition?.issues).toEqual([{ parameter: 'text', kind: 'unsupported' }]);
    const resolved = suspendCapabilityTransitionIssues(switched);
    expect(valid(resolved)).toBe(true);
    expect(params(resolved)).toEqual({ response_format: { type: 'json_object' }, thinking: { budget: 9 } });
    expect(params(changeCapabilityProtocol(resolved, responses))).toEqual({
      text: { format: { type: 'json_object' }, vendor_style: { compact: true } }, thinking: { budget: 9 },
    });
  });

  test('a format conflict uses the target only after explicit choice and retains the old schema for restoration', () => {
    const switched = changeCapabilityProtocol(cap({ response_format: { type: 'json_object' }, text: { format: { type: 'text' } } }), responses);
    expect(valid(switched)).toBe(false);
    const resolved = suspendCapabilityTransitionIssues(switched);
    expect(params(resolved)).toEqual({ text: { format: { type: 'text' } } });
    expect(valid(resolved)).toBe(true);
    const returned = changeCapabilityProtocol(resolved, chat);
    expect(params(returned)).toEqual({ response_format: { type: 'text' } });
    expect(returned.protocolParameterStash?.[chat]).toEqual({ response_format: { type: 'json_object' } });
    expect(params(restoreCapabilityProtocolParams(returned))).toEqual({ response_format: { type: 'json_object' } });
  });

  test('invalid JSON is never reformatted or discarded and can return to its original protocol', () => {
    const original = { ...cap({}), providerParamsJson: '{  "unfinished":' };
    const switched = changeCapabilityProtocol(original, responses);
    expect(switched.providerParamsJson).toBe(original.providerParamsJson);
    expect(switched.protocolParameterTransition?.invalidJson).toBe(true);
    const returned = changeCapabilityProtocol(switched, chat);
    expect(returned.providerParamsJson).toBe(original.providerParamsJson);
    expect(returned.protocolParameterTransition).toBeUndefined();
  });

  test('null reasoning is never restored into active parameters after a roundtrip', () => {
    const switched = changeCapabilityProtocol(cap({ reasoning: null }, responses), chat);
    const returned = changeCapabilityProtocol(switched, responses);
    expect(params(returned)).not.toHaveProperty('reasoning');
    expect(params(restoreCapabilityProtocolParams(returned))).not.toHaveProperty('reasoning');
  });

  test('successful conversion ends its old review context before subsequent target-protocol edits', () => {
    const switched = changeCapabilityProtocol(cap({ verbosity: 'low' }), responses);
    const edited = patchCapabilityDraft(switched, { providerParamsJson: '{"max_tokens_field":"max_tokens"}' });
    expect(params(edited)).toEqual({ max_tokens_field: 'max_tokens' });
  });

  test('compatibility shortcuts share conversion and native switches suspend only known scoped controls', () => {
    const source = cap({ stream_options: { include_usage: false }, max_tokens_field: 'max_tokens', verbosity: 'high', thinking: { vendor: true } });
    const explicit = applyProviderCompatibilityMode(definition(source), 'openai-responses', true).capabilities[0];
    expect(params(explicit)).toEqual({ text: { verbosity: 'high' }, thinking: { vendor: true } });
    const native = changeCapabilityProtocol(source, 'anthropic.messages');
    expect(params(native)).toEqual({ verbosity: 'high', thinking: { vendor: true } });
    expect(params(changeCapabilityProtocol(native, chat))).toEqual(params(source));
  });

  test('protocol conversion owns migrated defaults but preserves numeric defaults and context semantics', () => {
    const source = cap({ verbosity: 'low', reasoning_effort: 'medium', _nomifun_context_limit_kind: 'input_only' });
    source.modelDefaults = { model: 'a', contextLimit: 123, outputLimit: 99, providerParams: params(source) };
    expect(changeCapabilityProtocol(source, responses).modelDefaults).toEqual({ model: 'a', contextLimit: 123, outputLimit: 99,
      providerParams: { _nomifun_context_limit_kind: 'input_only' } });
  });

  test('applying a verified protocol preserves source and target caches without changing any tested fields', () => {
    const current = cap({ temperature: 0.2, stream_options: null, max_tokens_field: 'max_completion_tokens' });
    current.protocolParameterStash = { [responses]: { include: ['reasoning.encrypted_content'] } };
    const value = definition(current);
    const selected = { task: 'chat' as const, protocol: responses, connection_role: 'default', provider_params: { temperature: 0.2 },
      endpoint: '/responses', output_limit: 16_384 };
    const applied = applyDetectedChatProtocol(value, chatProtocolDetectionSnapshot(value.model, current), selected).capabilities[0];
    expect(capabilityInputsFromDefinition(definition(applied))![0]).toEqual(selected);
    expect(applied.protocolParameterStash).toEqual({ [chat]: { stream_options: null, max_tokens_field: 'max_completion_tokens' },
      [responses]: { include: ['reasoning.encrypted_content'] } });
    expect(applied.protocolParameterTransition?.restoredKeys).toEqual([]);
    expect(params(changeCapabilityProtocol(applied, chat))).toMatchObject({ stream_options: null, max_tokens_field: 'max_completion_tokens' });
  });

  test('an internally conflicting cache stays suspended until explicit restore and review', () => {
    const current = cap({ temperature: 0.2 }, responses);
    current.protocolParameterStash = { [chat]: { reasoning_effort: 'high', reasoning: { effort: 'low' } } };
    const switched = changeCapabilityProtocol(current, chat);
    expect(params(switched)).toEqual({ temperature: 0.2 });
    expect(switched.protocolParameterStash?.[chat]).toEqual(current.protocolParameterStash[chat]);
    const restored = restoreCapabilityProtocolParams(switched);
    expect(valid(restored)).toBe(false);
    expect(restored.protocolParameterTransition?.issues).toEqual([{ parameter: 'reasoning.effort', kind: 'conflict' }]);
  });
});
