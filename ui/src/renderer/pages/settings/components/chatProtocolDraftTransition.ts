import type { ModelCapabilityDraft } from './providerModelAdvanced';
import { equalProtocolParamValue, isOpenAIChatProtocol, transitionChatProtocolParams, type ChatProtocolTransitionIssue } from './chatProtocolTransition';

export interface ProtocolParameterTransitionNotice {
  source: string;
  target: string;
  issues: ChatProtocolTransitionIssue[];
  suspendedKeys: string[];
  restoredKeys: string[];
  mappedKeys: string[];
  invalidJson?: boolean;
}
export type ProtocolParameterStash = Record<string, Record<string, unknown>>;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const owns = (value: Record<string, unknown>, key: string): boolean => Object.prototype.hasOwnProperty.call(value, key);
const parse = (raw: string): Record<string, unknown> | undefined => {
  try { const value: unknown = raw.trim() ? JSON.parse(raw) : {}; return object(value) ? value : undefined; } catch { return undefined; }
};
const write = (params: Record<string, unknown>): string => Object.keys(params).length ? JSON.stringify(params, null, 2) : '';
const atomic = (path: string): boolean => ['response_format', 'text.format', 'stream_options', 'include', 'chain_rounds',
  'require_reasoning_content', 'max_tokens_field', 'verbosity', 'text.verbosity', 'reasoning_effort', 'reasoning.effort'].includes(path);

/** Merge scoped settings without combining two distinct output schemas. */
function merge(active: Record<string, unknown>, cached: Record<string, unknown>, overwrite: boolean, prefix = ''): { params: Record<string, unknown>; remaining: Record<string, unknown>; restored: string[] } {
  const params = structuredClone(active);
  const remaining: Record<string, unknown> = {};
  const restored: string[] = [];
  for (const [key, value] of Object.entries(cached)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (!owns(params, key) || overwrite || equalProtocolParamValue(params[key], value)) {
      if (object(params[key]) && object(value) && !atomic(path)) {
        const nested = merge(params[key], value, overwrite, path);
        params[key] = nested.params; restored.push(...nested.restored);
        if (Object.keys(nested.remaining).length) remaining[key] = nested.remaining;
      } else { params[key] = structuredClone(value); restored.push(path); }
    } else if (object(params[key]) && object(value) && !atomic(path)) {
      const nested = merge(params[key], value, overwrite, path);
      params[key] = nested.params; restored.push(...nested.restored);
      if (Object.keys(nested.remaining).length) remaining[key] = nested.remaining;
    } else remaining[key] = structuredClone(value);
  }
  return { params, remaining, restored };
}
function suspendScopedControls(params: Record<string, unknown>, target: string): Record<string, unknown> {
  const suspended: Record<string, unknown> = {};
  for (const [key, supported] of [
    ['chain_rounds', target === 'openai.responses'],
    ['stream_options', target === 'openai.chat_text'],
    ['require_reasoning_content', target === 'openai.chat_text'],
    ['max_tokens_field', isOpenAIChatProtocol(target)],
  ] as const) if (!supported && owns(params, key)) { suspended[key] = params[key]; delete params[key]; }
  return suspended;
}
function stashValues(stash: ProtocolParameterStash, protocol: string, values: Record<string, unknown>) {
  if (Object.keys(values).length) stash[protocol] = merge(stash[protocol] ?? {}, values, true).params;
}
function migratedKeys(before: Record<string, unknown>, after: Record<string, unknown>, suspended: Record<string, unknown>, source: string, target: string): string[] {
  const keys: string[] = [];
  if (object(before.reasoning) && owns(before.reasoning, 'effort') && (!object(after.reasoning) || !owns(after.reasoning, 'effort'))) keys.push('reasoning.effort');
  if (source === target) return keys;
  if (target === 'openai.responses') {
    for (const key of ['response_format', 'verbosity']) if (owns(before, key) && !owns(after, key) && !owns(suspended, key)) keys.push(key);
  } else if (target === 'openai.chat_text' && object(before.text)) {
    for (const key of ['format', 'verbosity']) if (owns(before.text, key) && (!object(after.text) || !owns(after.text, key)) &&
      (!object(suspended.text) || !owns(suspended.text, key))) keys.push(`text.${key}`);
  }
  return keys;
}

/** Parameter-only state. Addresses, connection roles, limits and credentials are never cached here. */
export function transitionCapabilityParams(capability: ModelCapabilityDraft, target: string, options: { restoreTarget?: boolean } = {}): ModelCapabilityDraft {
  const source = capability.protocol.trim(); target = target.trim();
  if (source === target && capability.protocolParameterTransition?.issues.length) return refreshCapabilityTransitionIssues(capability);
  const parsed = parse(capability.providerParamsJson);
  if (!parsed) {
    if (capability.protocolParameterTransition?.invalidJson && target === capability.protocolParameterTransition.source) {
      return { ...capability, protocolParameterTransition: undefined };
    }
    return source === target ? capability : { ...capability, protocolParameterTransition: {
      source: capability.protocolParameterTransition?.invalidJson ? capability.protocolParameterTransition.source : source,
      target, issues: [], suspendedKeys: [], restoredKeys: [], mappedKeys: [], invalidJson: true,
    } };
  }
  const converted = transitionChatProtocolParams(parsed, source, target);
  if (source !== target) Object.assign(converted.suspended, suspendScopedControls(converted.params, target));
  const stash = structuredClone(capability.protocolParameterStash ?? {});
  stashValues(stash, source, converted.suspended);
  let params = converted.params;
  let restoredKeys: string[] = [];
  if (source !== target && options.restoreTarget !== false && stash[target]) {
    const normalized = transitionChatProtocolParams(stash[target], target, target);
    // A conflicting cache needs an explicit restore/review; never reintroduce
    // unresolved nested effort while silently discarding its review state.
    if (!normalized.issues.length) {
      const restored = merge(params, normalized.params, false);
      params = restored.params; restoredKeys = restored.restored;
      if (Object.keys(restored.remaining).length) stash[target] = restored.remaining;
      else delete stash[target];
    }
  }
  const mappedKeys = migratedKeys(parsed, converted.params, converted.suspended, source, target);
  const changed = !equalProtocolParamValue(parsed, params);
  const notice = converted.issues.length || Object.keys(converted.suspended).length || restoredKeys.length || mappedKeys.length
    ? { source, target, issues: converted.issues, suspendedKeys: Object.keys(converted.suspended), restoredKeys, mappedKeys }
    : undefined;
  return { ...capability,
    providerParamsJson: changed ? write(params) : capability.providerParamsJson,
    providerParamsSource: changed ? 'user' : capability.providerParamsSource,
    protocolParameterStash: Object.keys(stash).length ? stash : undefined,
    protocolParameterTransition: notice,
    // Explicit protocol conversion owns native parameters. Keep only automatic
    // context semantics paired with the still-automatic numeric model limit.
    ...(capability.modelDefaults ? { modelDefaults: { ...capability.modelDefaults,
      providerParams: owns(capability.modelDefaults.providerParams, '_nomifun_context_limit_kind')
        ? { _nomifun_context_limit_kind: capability.modelDefaults.providerParams._nomifun_context_limit_kind } : {},
    } } : {}),
  };
}

/** Editing an unrelated value cannot clear unresolved format/reasoning conflicts. */
export function refreshCapabilityTransitionIssues(capability: ModelCapabilityDraft): ModelCapabilityDraft {
  const notice = capability.protocolParameterTransition;
  if (!notice) return capability;
  if (!notice.issues.length && !notice.invalidJson) return capability;
  const params = parse(capability.providerParamsJson);
  if (!params) return { ...capability, protocolParameterTransition: { ...notice, invalidJson: true } };
  const result = transitionChatProtocolParams(params, notice.source, notice.target);
  const stash = structuredClone(capability.protocolParameterStash ?? {});
  stashValues(stash, notice.source, result.suspended);
  return { ...capability,
    providerParamsJson: equalProtocolParamValue(params, result.params) ? capability.providerParamsJson : write(result.params),
    protocolParameterStash: Object.keys(stash).length ? stash : undefined,
    protocolParameterTransition: { ...notice, invalidJson: undefined, issues: result.issues,
      suspendedKeys: [...new Set([...notice.suspendedKeys, ...Object.keys(result.suspended)])] },
  };
}

function takePath(params: Record<string, unknown>, path: string, suspended: Record<string, unknown>) {
  const keys = path.split('.');
  let source = params; let saved = suspended;
  for (const key of keys.slice(0, -1)) {
    if (!object(source[key])) return;
    source = source[key];
    if (!object(saved[key])) saved[key] = {};
    saved = saved[key] as Record<string, unknown>;
  }
  const leaf = keys.at(-1)!;
  if (owns(source, leaf)) { saved[leaf] = source[leaf]; delete source[leaf]; }
}

/** Explicitly choose target settings/defaults without destroying source semantic options. */
export function suspendCapabilityTransitionIssues(capability: ModelCapabilityDraft): ModelCapabilityDraft {
  const notice = capability.protocolParameterTransition;
  const params = parse(capability.providerParamsJson);
  if (!notice || !params || !notice.issues.length) return capability;
  const suspended: Record<string, unknown> = {};
  for (const issue of [...notice.issues].sort((left, right) => left.parameter.split('.').length - right.parameter.split('.').length)) takePath(params, issue.parameter, suspended);
  const stash = structuredClone(capability.protocolParameterStash ?? {});
  stashValues(stash, notice.source, suspended);
  const resolved = transitionChatProtocolParams(params, notice.source, notice.target);
  stashValues(stash, notice.source, resolved.suspended);
  return { ...capability, providerParamsJson: write(resolved.params), providerParamsSource: 'user', protocolParameterStash: stash,
    protocolParameterTransition: { ...notice, issues: resolved.issues,
      suspendedKeys: [...new Set([...notice.suspendedKeys, ...notice.issues.map(issue => issue.parameter)])] },
  };
}

/** Explicit restore may replace corresponding current values; automatic restore never does. */
export function restoreCapabilityProtocolParams(capability: ModelCapabilityDraft): ModelCapabilityDraft {
  const cached = capability.protocolParameterStash?.[capability.protocol];
  const active = parse(capability.providerParamsJson);
  if (!cached || !active) return capability;
  const normalized = transitionChatProtocolParams(cached, capability.protocol, capability.protocol);
  const params = merge(active, normalized.params, true).params;
  const stash = structuredClone(capability.protocolParameterStash ?? {}); delete stash[capability.protocol];
  const previous = capability.protocolParameterTransition;
  return refreshCapabilityTransitionIssues({ ...capability, providerParamsJson: write(params), providerParamsSource: 'user', protocolParameterStash: stash,
    protocolParameterTransition: previous?.issues.length ? { ...previous, restoredKeys: Object.keys(cached) }
      : { source: capability.protocol, target: capability.protocol,
        issues: normalized.issues, restoredKeys: Object.keys(cached), suspendedKeys: [], mappedKeys: [] },
  });
}
