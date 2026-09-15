import { useEffect, useMemo } from 'react';
import type { ModelTask } from '@/common/config/storage';
import { capabilityOf } from '@/common/utils/providerModels';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { creationImageSizePolicy } from './parameterPolicy';
import type { CreationDraftController } from './useCreationDraft';
import { emptyCreationDraft } from './useCreationDraft';
import { capabilityFor, filesForCreation, inputsForMode } from './types';
import { useConfig } from '@/renderer/hooks/config/useConfig';

export function initialGenerationModel<T extends { providerId: string; model: string }>(options: readonly T[], preferred?: { provider_id: string; model: string }): T | undefined {
  return options.find(option => option.providerId === preferred?.provider_id && option.model === preferred.model) || (options.length === 1 ? options[0] : undefined);
}

const inactiveDraft = emptyCreationDraft();
const noop = () => {};
export function useGenerationModel(controller: CreationDraftController | null, files: readonly string[] = []) {
  const { draft, update } = controller || { draft: inactiveDraft, update: noop };
  const mode = draft.mode || draft.lastMode;
  const capability = capabilityFor(mode, inputsForMode(mode, draft.references), filesForCreation(mode, files).length > 0);
  const task = (mode === 'music' ? 'music_generation' : mode === 'video' ? 'video_generation' : capability === 't2i' ? 'image_generation' : 'image_edit') as ModelTask;
  const [preferred] = useConfig(task === 'music_generation' ? 'models.default.musicGeneration' : task === 'video_generation' ? 'models.default.videoGeneration' : task === 'image_edit' ? 'models.default.imageEdit' : 'models.default.imageGeneration');
  const catalog = useModelsForTask(task);
  const options = useMemo(() => catalog.groups.flatMap(({ provider, models }) => models.map(model => ({
    providerId: provider.id, model, platform: provider.platform,
    label: modelDisplayLabel(model, provider.models.find(entry => entry.model === model)?.display_name),
    providerLabel: provider.name,
    protocol: capabilityOf(provider, model, task)?.protocol || '',
  }))), [catalog.groups, task]);
  const selection = draft.models[mode];
  const selected = options.find(option => option.providerId === selection?.providerId && option.model === selection.model);
  useEffect(() => {
    if (!draft.mode || selection || catalog.isLoading || catalog.error) return;
    const first = initialGenerationModel(options, preferred);
    if (!first) return;
    update(current => ({ ...current, models: { ...current.models, [mode]: { providerId: first.providerId, model: first.model } } }));
  }, [catalog.error, catalog.isLoading, draft.mode, mode, options, preferred, selection, update]);
  const sizePolicy = useMemo(() => creationImageSizePolicy(selected), [selected]);
  return { ...catalog, options, selected, task, capability, sizePolicy, ready: Boolean(selected) && !catalog.isLoading && !catalog.error };
}
