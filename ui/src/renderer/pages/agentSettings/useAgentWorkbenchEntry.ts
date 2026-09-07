import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { useEffect, useRef } from 'react';
import { useLocation } from 'react-router-dom';

/** Resolve a homepage entry after the library arrives, once per navigation. */
export function useAgentWorkbenchEntry({
  library,
  loading,
  openTemplate,
  openPreset,
}: {
  library: AgentPresetLibraryResponse | null;
  loading: boolean;
  openTemplate: (template: OfficialPresetTemplate) => void;
  openPreset: (preset: AgentPresetSummary) => Promise<void>;
}) {
  const location = useLocation();
  const handledLocation = useRef<string | null>(null);

  useEffect(() => {
    if (loading || !library || handledLocation.current === location.key) return;

    handledLocation.current = location.key;
    const params = new URLSearchParams(location.search);
    const preset = library.user_presets.find(
      (candidate) => candidate.preset_id === params.get('preset')
    );
    if (preset) {
      void openPreset(preset);
      return;
    }
    const template = library.official_templates.find(
      (candidate) => candidate.template_key === params.get('template')
    );
    if (template) openTemplate(template);
  }, [library, loading, location.key, location.search, openPreset, openTemplate]);
}
