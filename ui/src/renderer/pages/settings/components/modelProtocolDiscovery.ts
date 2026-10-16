import type { ProviderId } from '@/common/types/ids';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import type { ProviderConnectionInput } from '@/common/types/provider/providerConnection';
import type { ProviderModelCapabilityInput } from '@/common/types/provider/providerModel';
import type { CreateProviderRequest } from '@/common/types/provider/providerApi';
import { transitionCapabilityParams } from './chatProtocolDraftTransition';
import { equalProtocolParamValue, transitionChatProtocolParams } from './chatProtocolTransition';
import {
  capabilityDraftFromResponse, capabilityInputsFromDefinition,
  parseProviderParams,
  type ModelCapabilityDraft, type ModelDefinitionDraft,
} from './providerModelAdvanced';

/** Connection context is local to the form and never enters a global query cache. */
export interface ModelProtocolDiscoveryContext {
  providerId?: ProviderId;
  platform: string;
  baseUrl: string;
  authScheme: string;
  credentials?: ProviderCredentials;
  connections?: ProviderConnectionInput[];
  bedrockConfig?: CreateProviderRequest['bedrock_config'];
}

const stableValue = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(stableValue);
  if (value !== null && typeof value === 'object') return Object.fromEntries(
    Object.entries(value).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, stableValue(item)]),
  );
  return value;
};

/** Explicit clears and saved parameters must not be refilled by documentation defaults. */
export const modelProtocolDiscoveryAppliesDefaults = (capability: ModelCapabilityDraft): boolean =>
  capability.providerParamsSource !== 'user' && capability.transportSource !== 'persisted';

/** Parameter ownership changes execution policy even when the JSON stays empty. */
export const modelProtocolDiscoverySnapshot = (model: string, capability: ModelCapabilityDraft): string =>
  JSON.stringify(stableValue({ model: model.trim(), capability: capabilityInputsFromDefinition({ model, capabilities: [capability] })?.[0],
    apply_recommended_defaults: modelProtocolDiscoveryAppliesDefaults(capability) }));

export const modelProtocolDiscoveryContextSnapshot = (context: ModelProtocolDiscoveryContext): string =>
  JSON.stringify(stableValue(context));

/** Apply exactly the tested capability only while the tested local draft still exists. */
export const applyDiscoveredModelProtocol = (
  definition: ModelDefinitionDraft,
  snapshot: string,
  selected: ProviderModelCapabilityInput,
  selectionSource: 'automatic' | 'manual' = 'automatic',
): ModelDefinitionDraft => {
  const current = definition.capabilities.find(capability => capability.task === selected.task);
  if (!current || modelProtocolDiscoverySnapshot(definition.model, current) !== snapshot) return definition;
  // The returned capability is exactly what was tested. Preserve target-side
  // cache for an explicit later restore instead of applying untested settings.
  const transitioned = transitionCapabilityParams(current, selected.protocol, { restoreTarget: false });
  const draft = capabilityDraftFromResponse(selected);
  const ownsAutomaticTransport = selectionSource === 'automatic' && current.protocolSelectionSource !== 'manual' &&
    (current.transportSource === 'blank' || current.transportSource === 'recommendation' || current.protocolDiscoveryModel === definition.model.trim());
  let modelDefaults = current.protocol === selected.protocol ? current.modelDefaults : transitioned.modelDefaults;
  if (ownsAutomaticTransport) {
    const before = parseProviderParams(transitioned.providerParamsJson);
    const after = parseProviderParams(draft.providerParamsJson);
    if (before.ok && after.ok) {
      const previousOwned = transitionChatProtocolParams(current.modelDefaults?.providerParams ?? {}, current.protocol, selected.protocol).params;
      const owned = Object.fromEntries(Object.entries(after.value).filter(([key, value]) =>
        !Object.prototype.hasOwnProperty.call(before.value, key) || (Object.prototype.hasOwnProperty.call(previousOwned, key) && equalProtocolParamValue(previousOwned[key], value))));
      modelDefaults = { ...modelDefaults, model: definition.model.trim(), providerParams: owned };
    }
  }
  return {
    ...definition,
    capabilities: definition.capabilities.map(capability => capability.task === selected.task ? {
      ...capability,
      protocolParameterStash: transitioned.protocolParameterStash,
      protocolParameterTransition: transitioned.protocolParameterTransition,
      modelDefaults,
      ...draft,
      routeSource: capability.routeSource,
      transportSource: 'user',
      providerParamsSource: 'user',
      protocolSelectionSource: selectionSource === 'manual' || (capability.protocolSelectionSource === 'manual' && capability.protocol === selected.protocol)
        ? 'manual' : 'automatic',
      protocolDiscoveryModel: ownsAutomaticTransport ? definition.model.trim() : undefined,
    } : capability),
  };
};
