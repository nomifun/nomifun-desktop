import type { ProviderId } from '@/common/types/ids';

export type ModelConfigurationTarget = {
  providerId: string;
  model: string;
};

const TARGET_PROVIDER_PARAM = 'provider';
const TARGET_MODEL_PARAM = 'model';
const TARGET_FOCUS_PARAM = 'focus';
const CAPABILITIES_FOCUS = 'capabilities';

/** Deep-link directly to one model's capability editor. */
export const modelCapabilityConfigurationRoute = (
  providerId: ProviderId,
  model: string,
): string => {
  const params = new URLSearchParams({
    section: 'models',
    [TARGET_PROVIDER_PARAM]: providerId,
    [TARGET_MODEL_PARAM]: model,
    [TARGET_FOCUS_PARAM]: CAPABILITIES_FOCUS,
  });
  return `/models?${params.toString()}`;
};

export const modelConfigurationTarget = (
  searchParams: URLSearchParams,
): ModelConfigurationTarget | null => {
  if (searchParams.get(TARGET_FOCUS_PARAM) !== CAPABILITIES_FOCUS) return null;
  const providerId = searchParams.get(TARGET_PROVIDER_PARAM)?.trim();
  const model = searchParams.get(TARGET_MODEL_PARAM)?.trim();
  return providerId && model ? { providerId, model } : null;
};

/** Keep unrelated Model Hub state while consuming the one-shot editor target. */
export const withoutModelConfigurationTarget = (
  searchParams: URLSearchParams,
): URLSearchParams => {
  const next = new URLSearchParams(searchParams);
  next.delete(TARGET_PROVIDER_PARAM);
  next.delete(TARGET_MODEL_PARAM);
  next.delete(TARGET_FOCUS_PARAM);
  return next;
};
