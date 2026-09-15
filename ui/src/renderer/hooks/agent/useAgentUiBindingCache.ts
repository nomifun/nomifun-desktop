import { unstable_serialize, useSWRConfig } from 'swr';
import type { AgentPresetUiBinding } from '@/common/types/pluginRuntimePlatform';

/** Preset consent has two read routes but one cache update policy. */
export function useAgentUiBindingCache() {
  const { cache, mutate } = useSWRConfig();
  return async (next: AgentPresetUiBinding) => {
    // Refresh aliases without making a successful save wait on another read.
    // Active pages retain their current selection; inactive pages reread on entry.
    void mutate(
      key => Array.isArray(key) && key[0] === 'agent-session-ui-binding' &&
        cache.get(unstable_serialize(key))?.data?.preset_id === next.preset_id,
    ).catch(() => { /* Each mounted reader exposes its own read error. */ });
    await mutate(['agent-preset-ui-binding', next.preset_id], next, { revalidate: false });
  };
}
