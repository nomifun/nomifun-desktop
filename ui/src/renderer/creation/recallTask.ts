import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { creationModeFor, type ConversationCreationTask, type CreationDraft, type CreationMode } from './types';
import { creationUserParameters } from './parameterPolicy';

export function recallCreationTask(draft: CreationDraft, task: ConversationCreationTask, assets: CreativeAsset[], target: CreationMode, useResult = false): CreationDraft {
  const originalMode = creationModeFor(task.capability);
  if (!originalMode) throw new Error('语音合成任务不属于音乐生成模式');
  const selectedAssets = useResult && target === 'video' ? assets.slice(0, 1) : assets;
  return {
    ...draft,
    pendingPrompt: String(task.params.prompt || ''),
    models: { ...draft.models, [target]: target === originalMode ? { providerId: task.provider_id, model: task.model } : draft.models[target] },
    parameters: { ...draft.parameters, [target]: target === originalMode ? creationUserParameters(target, task.params) : draft.parameters[target] },
    references: selectedAssets.filter(asset => !asset.deletedAt).map(asset => ({
      asset_id: asset.id, kind: asset.kind, title: asset.title, url: asset.thumbnailUrl || asset.originalUrl,
      role: useResult ? target === 'video' ? 'first_frame' : 'reference' : task.inputs?.find(input => input.asset_id === asset.id)?.role || 'reference',
    })),
  };
}
