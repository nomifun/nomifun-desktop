import { useConfig } from '@/renderer/hooks/config/useConfig';

/**
 * Returns whether auto-preview for newly created Office files is enabled globally.
 */
export const useAutoPreviewOfficeFilesEnabled = (): boolean => {
  const [enabled] = useConfig('system.autoPreviewOfficeFiles');
  return enabled ?? true;
};
