import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const sourceFile = (name: string) =>
  readFileSync(new URL(`./${name}`, import.meta.url), 'utf8');

describe('Agent Workbench user preset deletion', () => {
  test('renders a confirmed delete action only in the user preset list', () => {
    const library = sourceFile('AgentPresetLibrary.tsx');
    const userListStart = library.indexOf('presets.map');
    const deleteAction = library.indexOf('<Popconfirm', userListStart);

    expect(userListStart).toBeGreaterThanOrEqual(0);
    expect(deleteAction).toBeGreaterThan(userListStart);
    expect(library.slice(0, userListStart).includes('<Popconfirm')).toBe(false);
    expect(library.includes('onDeletePreset(preset)')).toBe(true);
    expect(library.includes("icon={<Delete theme='outline'")).toBe(true);
    expect(library.includes("cancelText={t('common.cancel')}")).toBe(true);
    expect(library.includes("okButtonProps={{ status: 'danger' }}")).toBe(true);
  });

  test('deletes through the mainline port, clears the current editor, and reloads', () => {
    const controller = sourceFile('useAgentSettingsController.ts');
    const deleteStart = controller.indexOf('const deletePreset =');
    const setDraftStart = controller.indexOf('const setDraft =', deleteStart);
    const deleteBlock = controller.slice(deleteStart, setDraftStart);

    expect(deleteBlock.includes('.deletePreset')).toBe(true);
    expect(
      deleteBlock.includes(
        'agentPlatform.deletePreset.invoke({ preset_id: preset.preset_id })'
      )
    ).toBe(true);
    expect(controller.includes('const clearEditorState = useCallback')).toBe(true);
    expect(deleteBlock.includes('clearEditorState()')).toBe(true);
    expect(deleteBlock.includes('await refreshPresetLibraries()')).toBe(true);
    expect(controller.includes('mutate(AGENT_PRESET_LIBRARY_SWR_KEY)')).toBe(true);
  });

  test('does not preserve a selection that disappeared from the reloaded library', () => {
    const controller = sourceFile('useAgentSettingsController.ts');

    expect(controller.includes('nextLibrary.user_presets.find')).toBe(true);
    expect(controller.includes('nextLibrary.official_templates.find')).toBe(true);
    expect(controller.includes('if (current) return current')).toBe(false);
  });

  test('wires the controller action and row busy state through the page', () => {
    const page = sourceFile('AgentSettingsPage.tsx');

    expect(page.includes('deletingPresetId={controller.deletingPresetId}')).toBe(true);
    expect(page.includes('controller.deletePreset(preset)')).toBe(true);
  });
});
