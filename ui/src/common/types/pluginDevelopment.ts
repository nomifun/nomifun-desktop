/** The whole authoring module; these are not user-created Plugin identities. */
export const PLUGIN_DEVELOPMENT_MODULE = 'plugin.development';
export const PLUGIN_CREATE_ACTIONS = [
  'list', 'open', 'read', 'plan', 'apply', 'check', 'preview', 'test_action', 'test_ui', 'install', 'inspect',
].map(action => `${PLUGIN_DEVELOPMENT_MODULE}/${action}`);

export interface PluginDeliveryRequirement {
  draft_id?: string;
  expected_count?: number;
}

export interface PluginDevelopmentPreflight {
  status: 'ready' | 'configure_agent' | 'unavailable';
  owner_user_id: string;
  reason: string;
  selection: { kind: 'preset'; presetId: string } | { kind: 'template'; templateKey: string };
  missing_actions?: string[];
  required_actions?: string[];
}

export interface PluginUiStep {
  operation: 'click' | 'fill' | 'text' | 'count' | 'reopen' | 'ready';
  selector?: string;
  value?: string | number;
}

export interface PluginUiCommand {
  test_token: string;
  draft_id: string;
  descriptor: import('./pluginPlatform').PluginSurfaceDescriptor;
  steps: PluginUiStep[];
  case_name: string;
}
