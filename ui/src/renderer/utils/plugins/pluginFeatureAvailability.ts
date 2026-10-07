import { PLUGIN_DEVELOPMENT_MODULE } from '@/common/types/pluginDevelopment';

// Shared visibility switch for plugin surfaces; existing Agent grants are preserved.
export const PLUGIN_FEATURE_VISIBLE = false;

export const isVisibleAgentModule = (moduleId: string): boolean =>
  PLUGIN_FEATURE_VISIBLE || moduleId !== PLUGIN_DEVELOPMENT_MODULE;
