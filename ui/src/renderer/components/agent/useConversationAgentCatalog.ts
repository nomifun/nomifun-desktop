import { useMemo } from 'react';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { conversationAgentPresets, isConversationAgentTemplate } from './conversationAgentCatalog';
import type { ExecutableAgentPreset } from './types';

/** The same workbench catalog for every ordinary Agent conversation ingress. */
export function useConversationAgentCatalog() {
  const catalog = useAgentPresets();
  const personal = useMemo(() => catalog.library ? conversationAgentPresets(catalog.library) : [],
    [catalog.library]);
  const presets = useMemo(() => personal.filter((preset): preset is ExecutableAgentPreset => Boolean(preset.current_stable_revision)), [personal]);
  const draftPresets = useMemo(() => personal.filter(preset => !preset.current_stable_revision), [personal]);
  const officialTemplates = useMemo(() => (catalog.library?.official_templates ?? []).filter(isConversationAgentTemplate), [catalog.library?.official_templates]);
  return { ...catalog, presets, draftPresets, officialTemplates };
}
