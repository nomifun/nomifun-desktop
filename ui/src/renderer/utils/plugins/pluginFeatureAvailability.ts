import { PLUGIN_DEVELOPMENT_MODULE } from '@/common/types/pluginDevelopment';

// Temporarily hide plugin UI while retaining its implementation and saved Agent grants.
export const PLUGIN_FEATURE_VISIBLE = false;

export const isVisibleAgentModule = (moduleId: string): boolean =>
  PLUGIN_FEATURE_VISIBLE || moduleId !== PLUGIN_DEVELOPMENT_MODULE;
