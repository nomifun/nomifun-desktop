import { isBackendHttpError } from '@/common/adapter/httpBridge';
import {
  pickerKindForResourceKind,
  type AgentResourceSelectionValue,
} from '@/renderer/hooks/agent/agentResourceSelection';

export type AgentSwitchFailureKind =
  | 'model_incompatible'
  | 'resources_unavailable'
  | 'capabilities_unavailable'
  | 'unknown';

const backendCodes = (error: unknown): string[] => {
  if (!isBackendHttpError(error)) return [];
  const codes = error.code ? [error.code] : [];
  const details = error.details as { diagnostics?: Array<{ code?: unknown }> } | undefined;
  for (const diagnostic of details?.diagnostics ?? []) {
    if (typeof diagnostic.code === 'string') codes.push(diagnostic.code);
  }
  return codes;
};

export const agentSwitchRequiresWebSearchModel = (error: unknown): boolean => {
  if (!isBackendHttpError(error) || error.code !== 'MODEL_ROUTE_FEATURES_MISSING') {
    return false;
  }
  const details = error.details as {
    missing_features?: unknown;
    required_protocol?: unknown;
  } | undefined;
  return details?.required_protocol === 'openai.responses'
    || (Array.isArray(details?.missing_features)
      && details.missing_features.includes('WebSearch'));
};

export const classifyAgentSwitchError = (error: unknown): AgentSwitchFailureKind => {
  const codes = backendCodes(error);
  if (codes.some((code) => /RESOURCE_(?:SELECTION|BINDING|OPERATION)/.test(code))) {
    return 'resources_unavailable';
  }
  if (codes.some((code) => /MODEL_|CHAT_ROUTE|PROVIDER_/.test(code))) {
    return 'model_incompatible';
  }
  if (codes.some((code) => /CAPABILITY_|ROLE_COVERAGE|RUNTIME_FEATURE/.test(code))) {
    return 'capabilities_unavailable';
  }
  return 'unknown';
};

type PersistedResourceBinding = {
  resource_kind?: unknown;
  resource_id?: unknown;
};

/** Recover reusable user choices from the current frozen Session binding. */
export const resourceSelectionValueFromSessionExtra = (
  extra: unknown
): AgentResourceSelectionValue => {
  const bindings = (
    extra as {
      nomi_core_session?: {
        binding?: { typed_resource_bindings?: PersistedResourceBinding[] };
      };
    } | undefined
  )?.nomi_core_session?.binding?.typed_resource_bindings;
  const value: AgentResourceSelectionValue = {};
  for (const binding of bindings ?? []) {
    if (typeof binding.resource_kind !== 'string' || typeof binding.resource_id !== 'string') {
      continue;
    }
    const pickerKind = pickerKindForResourceKind(binding.resource_kind);
    if (pickerKind) value[pickerKind] = binding.resource_id;
  }
  return value;
};
