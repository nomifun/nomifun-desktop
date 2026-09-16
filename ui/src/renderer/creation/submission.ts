import { uuidv7 } from '@/common/utils';
import type { AgentPresetId } from '@/common/types/agentPlatform';
import type { CreationDraft, CreationParameters, SubmitCreationRequest } from './types';
import { capabilityFor, filesForCreation, inputsForMode } from './types';
import type { ImageGenerationModelOption } from './parameters/image';
import { creationImageSizePolicy, creationVideoInputRoles, normalizeCreationParameters } from './parameterPolicy';
import { browserStorageGenerationKey } from '@/common/utils/browserStorageKey';

/** Freeze the actual provider payload at admission, independently of the chat model. */
export function buildCreationRequest(draft: CreationDraft, prompt: string, presetId: AgentPresetId, files: string[] = [], option?: ImageGenerationModelOption): SubmitCreationRequest {
  const mode = draft.mode;
  if (!mode || !prompt.trim()) throw new Error('请先填写创作描述');
  const model = draft.models[mode];
  if (!model) throw new Error('请选择生成模型');
  const inputs = inputsForMode(mode, draft.references);
  const referenceFiles = filesForCreation(mode, files);
  if (mode === 'video' && option) {
    if (inputs.some(input => !creationVideoInputRoles(option).includes(input.role))) throw new Error('当前视频模型不支持所选素材角色，请调整后生成');
    if (option.protocol === 'openai.videos' && inputs.length + referenceFiles.length > 1) throw new Error('当前视频模型只支持一张首帧参考图，请调整素材');
  }
  const params: CreationParameters = { ...normalizeCreationParameters(mode, draft.parameters[mode], option || model), prompt };
  if (mode === 'image') {
    const policy = creationImageSizePolicy(option || model);
    const size = policy.options.find(value => !value.disabled && ((params.size !== undefined && value.requestSize === params.size) || (params.aspect !== undefined && value.value === params.aspect))) || policy.options.find(value => !value.disabled);
    if (size?.value === 'auto') {
      delete params.size; delete params.width; delete params.height;
    } else if (size) Object.assign(params, { size: size.requestSize || size.value, width: size.width, height: size.height });
    delete params.aspect;
  } else if (mode === 'music') {
    params.instrumental = params.instrumental !== false;
    if (params.instrumental) delete params.lyrics;
  }
  return { provider_id: model.providerId, model: model.model, capability: capabilityFor(mode, inputs, referenceFiles.length > 0), params, inputs, preset_id: presetId, ...(referenceFiles.length ? { files: referenceFiles } : {}) };
}

type PendingAttempt = { fingerprint: string; key: string };
const attempts = new Map<string, PendingAttempt[]>();
const attemptStorageKey = (scope: string) => browserStorageGenerationKey(`creation-admission:${scope}`);
function readAttempts(storageKey: string): PendingAttempt[] {
  const cached = attempts.get(storageKey);
  if (cached) return cached;
  try {
    const stored: unknown = JSON.parse(sessionStorage.getItem(storageKey) || 'null');
    if (Array.isArray(stored)) return stored.filter(value => typeof value?.fingerprint === 'string' && typeof value?.key === 'string');
  } catch { /* Memory fallback. */ }
  return [];
}
/** An uncertain network retry reuses its admission key, including after navigation. */
export function creationAttempt(scope: string, request: SubmitCreationRequest): string {
  const fingerprint = JSON.stringify(request);
  const storageKey = attemptStorageKey(scope);
  const previous = readAttempts(storageKey);
  const existing = previous.find(value => value.fingerprint === fingerprint);
  if (existing) return existing.key;
  const next = { fingerprint, key: uuidv7() };
  const pending = [...previous, next];
  attempts.set(storageKey, pending);
  try { sessionStorage.setItem(storageKey, JSON.stringify(pending)); } catch { /* Memory fallback. */ }
  return next.key;
}

export function acknowledgeCreationAttempt(scope: string, key: string): void {
  const storageKey = attemptStorageKey(scope);
  const remaining = readAttempts(storageKey).filter(value => value.key !== key);
  attempts.set(storageKey, remaining);
  try {
    if (remaining.length) sessionStorage.setItem(storageKey, JSON.stringify(remaining));
    else sessionStorage.removeItem(storageKey);
  } catch { /* The accepted receipt is authoritative. */ }
}
