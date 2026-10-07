import type { NavigateFunction } from 'react-router-dom';

export interface PluginAuthoringOptions {
  plugin_id?: string;
  expected_plugin_revision?: number;
  draft_id?: string;
  template?: 'agent.before_tool';
}

/** Navigation carries identities only. The authoring page owns admission and input. */
export function pluginAuthoringPath(options: PluginAuthoringOptions = {}): string {
  const search = new URLSearchParams();
  if (options.plugin_id) search.set('plugin_id', options.plugin_id);
  if (options.expected_plugin_revision !== undefined) search.set('expected_plugin_revision', String(options.expected_plugin_revision));
  if (options.draft_id) search.set('draft_id', options.draft_id);
  if (options.template) search.set('template', options.template);
  return `/plugins/create${search.size ? `?${search}` : ''}`;
}

export async function launchPluginAuthoring(navigate: NavigateFunction, options: PluginAuthoringOptions = {}) {
  await navigate(pluginAuthoringPath(options));
}
