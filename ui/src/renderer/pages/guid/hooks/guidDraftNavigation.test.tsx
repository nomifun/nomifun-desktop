import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { uuidv7 } from '@/common/utils';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { configService } from '@/common/config/configService';
import type { AgentPresetSummary, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { parseProviderId } from '@/common/types/ids';
import { useGuidCreation } from '@/renderer/creation/useGuidCreation';
import { useGuidInput } from './useGuidInput';
import { useGuidAgentSelection } from './useGuidAgentSelection';

const templates = ['chat.minimal', 'assistant.general', 'creative-studio.default'].map(template_key => ({ template_key })) as OfficialPresetTemplate[];
const stablePreset = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
  source: 'user', display_name: 'Release reviewer', bound_target_count: 0,
  current_stable_revision: { preset_id: '0190f5fe-7c00-7a00-8000-000000000101', revision: 1, revision_digest: 'a'.repeat(64) },
} as AgentPresetSummary;
beforeEach(() => { configService.reset(); setBrowserStorageGeneration(uuidv7()); });
afterEach(() => { cleanup(); sessionStorage.clear(); configService.reset(); });

function mountDraft(
  initialEntry = '/guid',
  agentOptions: Parameters<typeof useGuidAgentSelection>[0] = {},
) {
  let composer!: {
    input: ReturnType<typeof useGuidInput>;
    agent: ReturnType<typeof useGuidAgentSelection>;
    creation: ReturnType<typeof useGuidCreation>;
  };
  function Composer() {
    const input = useGuidInput({ locationState: null });
    const agent = useGuidAgentSelection(agentOptions);
    const creation = useGuidCreation(agent, input.input, input.files, input.dir);
    composer = { input, agent, creation };
    return <textarea aria-label='草稿' value={input.input} onChange={e => input.setInput(e.target.value)} />;
  }
  function Navigation() {
    const navigate = useNavigate();
    return <><button onClick={() => navigate('/assets')}>资产库</button><button onClick={() => navigate('/guid')}>会话</button><button onClick={() => navigate(-1)}>返回</button></>;
  }
  const page = render(<MemoryRouter initialEntries={[initialEntry]}><SWRConfig value={{ provider: () => new Map(),
    fallback: { providers: [], 'agent-presets.library': { official_templates: templates, user_presets: [stablePreset], active_bindings: [] } },
    revalidateOnMount: false, shouldRetryOnError: false,
  }}><Navigation /><Routes><Route path='/guid' element={<Composer />} /><Route path='/assets' element={<div>资产列表</div>} /></Routes></SWRConfig></MemoryRouter>);
  return { page, current: () => composer };
}

test('new Guid conversations use the workbench default without persisting draft-only switches', () => {
  configService.setLocal('guid.defaultAgentSelection', {
    kind: 'preset',
    presetId: stablePreset.preset_id,
  });
  const { current } = mountDraft('/guid', { resetAgentSelection: true });

  expect(current().agent.selection).toEqual({
    kind: 'preset',
    presetId: stablePreset.preset_id,
  });
  act(() => current().agent.setSelection({
    kind: 'template',
    templateKey: 'chat.minimal',
  }));
  expect(current().agent.selection).toEqual({
    kind: 'template',
    templateKey: 'chat.minimal',
  });
  expect(configService.get('guid.defaultAgentSelection')).toEqual({
    kind: 'preset',
    presetId: stablePreset.preset_id,
  });
  expect(configService.get('guid.agentSelection')).toBeUndefined();
});

test.each(['image', 'video', 'music'] as const)('%s draft survives page unmount with text, files, workspace, Agent, references and parameters', mode => {
  const save = spyOn(configService, 'set').mockResolvedValue(undefined);
  try {
    const { page, current } = mountDraft();
    const model = { providerId: parseProviderId('0190f5fe-7c00-7a00-8000-000000000105'), model: 'chosen-model' };
    act(() => {
      current().input.setInput('342524542，继续创作');
      current().input.handleFilesUploaded(['C:/reference.png']);
      current().input.setDir('C:/project');
      current().creation.selectMode(mode);
    });
    act(() => current().creation.update(draft => ({ ...draft,
      models: { ...draft.models, [mode]: model },
      parameters: { ...draft.parameters, [mode]: { count: 2, aspect: '16:9' } },
      references: [{ asset_id: 'saved-reference', kind: 'image', title: '参考图', role: 'reference' }],
    })));
    const expectedCreation = structuredClone(current().creation.draft);
    for (let visit = 0; visit < 2; visit++) {
      fireEvent.click(page.getByRole('button', { name: '资产库' }));
      expect(page.queryByRole('textbox')).toBeNull();
      fireEvent.click(page.getByRole('button', { name: '会话' }));
      expect(current().input.input).toBe('342524542，继续创作');
      expect(current().input.files).toEqual(['C:/reference.png']);
      expect(current().input.dir).toBe('C:/project');
      expect(current().agent.selection).toEqual({ kind: 'template', templateKey: 'creative-studio.default' });
      expect(current().creation.draft).toEqual(expectedCreation);
    }
  } finally { save.mockRestore(); }
});

test('ordinary Agent draft and post-navigation successful-send cleanup are retained', () => {
  const save = spyOn(configService, 'set').mockResolvedValue(undefined);
  try {
    const { page, current } = mountDraft();
    act(() => {
      current().agent.setSelection({ kind: 'template', templateKey: 'assistant.general' });
      current().input.setInput('请解释这份文件');
      current().input.setFiles(previous => [...previous, 'C:/brief.pdf']);
      current().input.setFiles(previous => [...previous, 'C:/notes.txt']);
    });
    const sentInput = current().input;
    fireEvent.click(page.getByRole('button', { name: '资产库' }));
    fireEvent.click(page.getByRole('button', { name: '会话' }));
    expect(current().input.input).toBe('请解释这份文件');
    expect(current().input.files).toEqual(['C:/brief.pdf', 'C:/notes.txt']);
    expect(current().agent.selection).toEqual({ kind: 'template', templateKey: 'assistant.general' });
    fireEvent.click(page.getByRole('button', { name: '资产库' }));
    act(() => { sentInput.setInput(''); sentInput.setFiles([]); });
    fireEvent.click(page.getByRole('button', { name: '会话' }));
    expect(current().input.input).toBe('');
    expect(current().input.files).toEqual([]);
  } finally { save.mockRestore(); }
});

test('Back does not replay the original image entry after choosing video', () => {
  const save = spyOn(configService, 'set').mockResolvedValue(undefined);
  try {
    const { page, current } = mountDraft('/guid?creation=image');
    expect(current().creation.draft.mode).toBe('image');
    act(() => current().creation.selectMode('video'));
    fireEvent.click(page.getByRole('button', { name: '资产库' }));
    fireEvent.click(page.getByRole('button', { name: '返回' }));
    expect(current().creation.draft.mode).toBe('video');
  } finally { save.mockRestore(); }
});
