import { useMemo, useState } from 'react';
import type { TProviderWithModel } from '@/common/config/storage';
import { parseProviderId, type ProviderId } from '@/common/types/ids';
import { configService } from '@/common/config/configService';
import { capabilityOf, capabilitySupportsTechnicalCapability } from '@/common/utils/providerModels';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';

export type AgentLaunchModel = { provider_id: ProviderId; model: string };

/** Launch-local model choices never change an Agent or the installation default. */
export function useAgentLaunchModel({ initialModel, defaultModel, ready, requiresToolCalls }: {
  initialModel?: AgentLaunchModel;
  defaultModel?: { provider_id: string; model: string } | null;
  ready: boolean;
  requiresToolCalls: boolean;
}) {
  const catalog = useModelsForTask('chat');
  const [explicit, setExplicit] = useState<AgentLaunchModel | undefined>(initialModel);
  const installationDefault = configService.get('nomi.defaultModel');
  // An unavailable configured model remains visible until explicitly replaced.
  const inheritedModel = defaultModel
    ? { ...defaultModel, provider_id: parseProviderId(defaultModel.provider_id) }
    : installationDefault;
  const selection = explicit ?? (ready ? inheritedModel : undefined);
  const provider = catalog.groups.find(group => group.provider.id === selection?.provider_id);
  const available = Boolean(selection && provider?.models.includes(selection.model));
  const compatible = available && (!requiresToolCalls || capabilitySupportsTechnicalCapability(
    capabilityOf(provider?.provider, selection?.model ?? '', 'chat'), 'function_calling'));
  const currentModel = useMemo<TProviderWithModel | undefined>(() => selection && provider
    ? { ...provider.provider, use_model: selection.model } : undefined, [provider, selection?.provider_id, selection?.model]);
  return {
    ...catalog, selection, currentModel, available, compatible,
    reset: (model?: AgentLaunchModel) => setExplicit(model),
    select: (providerId: ProviderId, model: string) => setExplicit({ provider_id: providerId, model }),
  };
}
