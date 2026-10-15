import type { ProviderId } from '@/common/types/ids';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import type { ProviderConnectionInput } from '@/common/types/provider/providerConnection';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import {
  capabilityDraftFromResponse, capabilityInputsFromDefinition,
  type ModelCapabilityDraft, type ModelDefinitionDraft,
} from './providerModelAdvanced';

/** Connection context is local to the form and never enters a global query cache. */
export interface ModelProtocolDetectionContext {
  providerId?: ProviderId;
  platform: string;
  baseUrl: string;
  authScheme: string;
  credentials?: ProviderCredentials;
  connections?: ProviderConnectionInput[];
}

export const isDetectableChatProtocol = (protocol: string): boolean =>
  protocol === 'openai.responses' || protocol === 'openai.chat_text';

const stableValue = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(stableValue);
  if (value !== null && typeof value === 'object') return Object.fromEntries(
    Object.entries(value).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, stableValue(item)]),
  );
  return value;
};

/** Execution fields only; UI recommendation ownership does not invalidate a test. */
export const chatProtocolDetectionSnapshot = (model: string, capability: ModelCapabilityDraft): string =>
  JSON.stringify(stableValue({ model: model.trim(), capability: capabilityInputsFromDefinition({ model, capabilities: [capability] })?.[0] }));

export const chatProtocolDetectionContextSnapshot = (context: ModelProtocolDetectionContext): string =>
  JSON.stringify(stableValue(context));

/** Apply exactly the tested capability only while the tested local draft still exists. */
export const applyDetectedChatProtocol = (
  definition: ModelDefinitionDraft,
  snapshot: string,
  selected: ProviderModelCapabilityInput,
): ModelDefinitionDraft => {
  const current = definition.capabilities.find(capability => capability.task === 'chat');
  if (!current || selected.task !== 'chat' || !isDetectableChatProtocol(selected.protocol) ||
    chatProtocolDetectionSnapshot(definition.model, current) !== snapshot) return definition;
  return {
    ...definition,
    capabilities: definition.capabilities.map(capability => capability.task === 'chat' ? {
      ...capability,
      ...capabilityDraftFromResponse(selected),
      routeSource: 'user',
      transportSource: 'user',
      providerParamsSource: 'user',
    } : capability),
  };
};
