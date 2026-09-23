import { useCallback } from 'react';

type ModelSelectorProvider = {
  name?: string;
  platform?: string;
};

export const formatModelSelectorProviderLabel = (
  provider: ModelSelectorProvider,
): string => provider.name?.trim() || provider.platform?.trim() || '';

/** Provider label shared by every model-selection surface. */
export const useModelSelectorProviderLabel = () =>
  useCallback(formatModelSelectorProviderLabel, []);
