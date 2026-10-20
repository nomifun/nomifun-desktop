import '../../../../test/setup-dom.ts';
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import type { PropsWithChildren } from 'react';
import { SWRConfig } from 'swr';
import { asAgentPresetId, asDigestHex, asResolvedSnapshotId, type AgentPresetLibraryResponse, type AgentPresetSummary, type OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { AGENT_PRESET_LIBRARY_SWR_KEY, useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { useConversationAgentCatalog } from './useConversationAgentCatalog';

afterEach(cleanup);
const preset = (suffix: string, saved = true): AgentPresetSummary => {
  const preset_id = asAgentPresetId(`0190f5fe-7c00-7a00-8000-${suffix.padStart(12, '0')}`);
  return { preset_id, display_name: `Agent ${suffix}`, source: 'user', bound_target_count: 0,
    ...(saved ? { current_stable_revision: { preset_id, revision: 1, revision_digest: asDigestHex('a'.repeat(64)) } } : {}) };
};
const template = (template_key: OfficialPresetTemplate['template_key']): OfficialPresetTemplate => ({
  template_key, immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_resource_kinds: [], required_runtime_features: [] },
});

test('one shared catalog exposes unsaved official templates, personal saved/draft entries and exact product filtering', () => {
  const personal = preset('101'), draft = preset('102', false), customer = preset('103');
  const data: AgentPresetLibraryResponse = {
    official_templates: [template('assistant.general'), template('companion.default'), template('customer-service.default')],
    official_presets: { 'assistant.general': { ...preset('104', false), source: 'official' } },
    user_presets: [personal, draft, customer],
    active_bindings: [{ target_kind: 'customer', target_id: 'support', preset_revision_ref: customer.current_stable_revision!,
      resolved_snapshot_ref: { snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000105'), snapshot_digest: asDigestHex('b'.repeat(64)) }, binding_version: 1 }],
    fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 3, user_preset_count: 3 },
  };
  const cache = new Map();
  const wrapper = ({ children }: PropsWithChildren) => <SWRConfig value={{ provider: () => cache, revalidateOnMount: false,
    fallback: { [AGENT_PRESET_LIBRARY_SWR_KEY]: data } }}>{children}</SWRConfig>;
  const hook = renderHook(() => ({ shared: useConversationAgentCatalog(), original: useAgentPresets() }), { wrapper });
  expect(hook.result.current.shared.officialTemplates.map(item => item.template_key)).toEqual(['assistant.general']);
  expect(hook.result.current.shared.library?.official_presets['assistant.general']?.current_stable_revision).toBeUndefined();
  expect(hook.result.current.shared.presets).toEqual([personal]);
  expect(hook.result.current.shared.draftPresets).toEqual([draft]);
  expect(hook.result.current.original.presets).toEqual([personal, draft, customer]);
});
