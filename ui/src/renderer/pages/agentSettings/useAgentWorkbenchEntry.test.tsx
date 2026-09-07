import '../../../../test/setup-dom.ts';

import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import type { PropsWithChildren } from 'react';
import { MemoryRouter, useNavigate } from 'react-router-dom';
import { useAgentWorkbenchEntry } from './useAgentWorkbenchEntry';

const firstTemplate = { template_key: 'chat.minimal' } as OfficialPresetTemplate;
const requestedTemplate = { template_key: 'coding.codex' } as OfficialPresetTemplate;
const draft = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000102',
  display_name: 'Research assistant',
  source: 'user',
  bound_target_count: 0,
} as AgentPresetSummary;
const library = {
  official_templates: [firstTemplate, requestedTemplate],
  user_presets: [draft],
  active_bindings: [],
} as unknown as AgentPresetLibraryResponse;

const renderEntry = (entry: string, initialLibrary: AgentPresetLibraryResponse | null = library) => {
  const openedTemplates: OfficialPresetTemplate[] = [];
  const openedPresets: AgentPresetSummary[] = [];
  const openTemplate = (template: OfficialPresetTemplate) => { openedTemplates.push(template); };
  const openPreset = async (preset: AgentPresetSummary) => { openedPresets.push(preset); };
  const hook = renderHook(
    ({ currentLibrary, loading }) => {
      useAgentWorkbenchEntry({ library: currentLibrary, loading, openTemplate, openPreset });
      return useNavigate();
    },
    {
      initialProps: { currentLibrary: initialLibrary, loading: initialLibrary === null },
      wrapper: ({ children }: PropsWithChildren) => (
        <MemoryRouter initialEntries={[entry]}>{children}</MemoryRouter>
      ),
    }
  );
  return { ...hook, openedTemplates, openedPresets };
};

afterEach(cleanup);

describe('Agent Workbench homepage entries', () => {
  test('waits for the library and selects the requested template instead of the first row', () => {
    const entry = renderEntry('/agent?template=coding.codex', null);
    expect(entry.openedTemplates).toEqual([]);

    entry.rerender({ currentLibrary: library, loading: false });
    expect(entry.openedTemplates).toEqual([requestedTemplate]);
    expect(entry.openedPresets).toEqual([]);
  });

  test('opens the requested draft for editing', () => {
    const entry = renderEntry(`/agent?preset=${draft.preset_id}`);
    expect(entry.openedPresets).toEqual([draft]);
    expect(entry.openedTemplates).toEqual([]);
  });

  test('library refresh does not reopen the entry and discard subsequent editing', () => {
    const entry = renderEntry(`/agent?preset=${draft.preset_id}`);
    entry.rerender({ currentLibrary: { ...library }, loading: false });
    expect(entry.openedPresets).toEqual([draft]);
  });

  test('honors a later navigation while the workbench stays mounted', () => {
    const entry = renderEntry('/agent?template=coding.codex');
    act(() => { void entry.result.current('/agent?template=chat.minimal'); });
    expect(entry.openedTemplates).toEqual([requestedTemplate, firstTemplate]);
  });

  test('unknown or deleted entries leave the normal workbench selection intact', () => {
    const entry = renderEntry('/agent?preset=deleted&template=unknown');
    expect(entry.openedTemplates).toEqual([]);
    expect(entry.openedPresets).toEqual([]);
  });
});
