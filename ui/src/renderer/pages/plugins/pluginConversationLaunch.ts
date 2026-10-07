import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { uuidv7 } from '@/common/utils';
import { agentBrowserStorageGenerationKey } from '@/common/utils/browserStorageKey';
import { readGuidDefaultAgentSelection } from '../guid/hooks/agentSelectionUtils';
import type { NavigateFunction } from 'react-router-dom';

export interface PluginLaunchIntent {
  version: 1;
  token: string;
  owner_user_id: string;
  created_at: number;
  requirement?: string;
  files?: string[];
  plugin_id?: string;
  expected_plugin_revision?: number;
  draft_id?: string;
  template?: 'agent.before_tool';
}
const memory = new Map<string, PluginLaunchIntent>();
const key = (token: string) => agentBrowserStorageGenerationKey(`plugin-launch:${token}`);
const ttl = 24 * 60 * 60 * 1000;

export function readPluginLaunchIntent(token: string, owner: string): PluginLaunchIntent | null {
  let value: PluginLaunchIntent | null = memory.get(key(token)) ?? null;
  if (!value) {
    try { value = JSON.parse(sessionStorage.getItem(key(token)) ?? 'null') as PluginLaunchIntent | null; } catch { return null; }
  }
  if (!value || value.version !== 1 || value.token !== token || value.owner_user_id !== owner
      || Date.now() - value.created_at > ttl
      || (value.files !== undefined && (!Array.isArray(value.files) || !value.files.every(file => typeof file === 'string')))) return null;
  return value;
}
export function consumePluginLaunchIntent(token: string) {
  memory.delete(key(token));
  try { sessionStorage.removeItem(key(token)); } catch { /* Memory is the fallback. */ }
}

export async function launchPluginConversation(
  navigate: NavigateFunction,
  options: Partial<Pick<PluginLaunchIntent, 'requirement' | 'files' | 'plugin_id' | 'expected_plugin_revision' | 'draft_id' | 'template'>> = {},
  existingToken?: string,
) {
  const selection = readGuidDefaultAgentSelection();
  const preflight = await pluginPlatform.authoring.preflight.invoke({ selection });
  if (preflight.status === 'unavailable') throw new Error(preflight.reason);
  const previous = existingToken ? readPluginLaunchIntent(existingToken, preflight.owner_user_id) : null;
  if (existingToken && !previous) throw new Error('PLUGIN_LAUNCH_EXPIRED');
  const intent: PluginLaunchIntent = previous ? { ...previous, ...options } : {
    version: 1, token: uuidv7(), owner_user_id: preflight.owner_user_id, created_at: Date.now(), ...options,
  };
  memory.set(key(intent.token), intent);
  try { sessionStorage.setItem(key(intent.token), JSON.stringify(intent)); } catch { /* Retain memory draft. */ }
  const search = new URLSearchParams({ pluginIntent: intent.token });
  if (preflight.status === 'configure_agent') {
    search.set(selection.kind === 'preset' ? 'preset' : 'template',
      selection.kind === 'preset' ? selection.presetId : selection.templateKey);
    search.set('module', 'plugin.development');
    await navigate(`/agent?${search}`);
    return;
  }
  await navigate(`/guid?${search}`, { state: {
    resetAgentSelection: true,
    ...(selection.kind === 'preset' ? { selectedAgentPresetId: selection.presetId } : { selectedAgentTemplateKey: selection.templateKey }),
  } });
}
