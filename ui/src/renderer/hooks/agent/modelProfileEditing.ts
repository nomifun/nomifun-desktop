import type { ModelProfile, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ModelProfileUpsertRequest } from '@/common/types/provider/providerApi';
import type { ProviderId } from '@/common/types/ids';

/** Display order of modality/task options in model profile editors. */
export const MODEL_TASK_ORDER: ModelTask[] = [
  'chat',
  'image_generation',
  'image_edit',
  'video_generation',
  'speech_synthesis',
  'speech_recognition',
  'embedding',
  'rerank',
];

/** Display order of trait options in model profile editors. */
export const MODEL_TRAIT_ORDER: ModelTrait[] = [
  'vision_input',
  'function_calling',
  'reasoning',
  'web_search',
];

/**
 * Tasks shown in editors. Inferred profiles are VISIBLE (pre-checked with a
 * "system inferred" hint); saving converts them to `source='user'`.
 */
export const editableModelTasks = (profile?: ModelProfile): ModelTask[] => profile?.tasks ?? [];

/** Traits shown in editors — same visibility rule as {@link editableModelTasks}. */
export const editableModelTraits = (profile?: ModelProfile): ModelTrait[] => profile?.traits ?? [];

/** Whether the profile is a system-inferred (not yet user-confirmed) one. */
export const isInferredModelProfile = (profile?: ModelProfile): boolean => profile?.source === 'inferred';

/**
 * Model-row badges show ALL tasks including chat (chat renders as a small
 * neutral tag, non-chat tasks colored).
 */
export const visibleModelTaskBadges = (profile?: ModelProfile): ModelTask[] => editableModelTasks(profile);

/** Primary task of a profile — what the health probe should exercise. */
export const primaryModelTask = (profile?: ModelProfile): ModelTask | undefined => profile?.tasks?.[0];

export const buildModelProfileUpsertRequest = (
  providerId: ProviderId,
  model: string,
  tasks: ModelTask[],
  traits: ModelTrait[]
): ModelProfileUpsertRequest => ({
  provider_id: providerId,
  model,
  tasks,
  traits,
  source: 'user',
});
