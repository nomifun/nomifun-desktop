import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { SkillInfo } from '@/common/types/skill';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import * as arcoMessageHook from '@/renderer/utils/ui/useArcoMessage';
import SkillsLibraryPanel from './SkillsLibraryPanel';
import { useSkillsLibrary } from './useSkillsLibrary';

const locale = createInstance();
const restore: Array<() => void> = [];
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { common, settings } } } });
});
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

const skill = (name: string): SkillInfo => ({ name, description: `${name} description`, location: `/skills/${name}/SKILL.md`, is_custom: true, source: 'custom' });

function prepareMocks() {
  const message = spyOn(arcoMessageHook, 'useArcoMessage').mockReturnValue([
    { success: () => () => {}, error: () => () => {} }, null,
  ] as unknown as ReturnType<typeof arcoMessageHook.useArcoMessage>);
  const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([skill('installed')]);
  const paths = spyOn(ipcBridge.fs.getSkillPaths, 'invoke').mockResolvedValue({ user_skills_dir: '/skills', builtin_skills_dir: '/builtin' });
  const auto = spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  const picker = spyOn(ipcBridge.dialog.showOpen, 'invoke');
  const importing = spyOn(ipcBridge.fs.importSkillWithSymlink, 'invoke').mockResolvedValue({ skill_name: 'imported' });
  const sources = spyOn(ipcBridge.fs.detectAndCountExternalSkills, 'invoke').mockResolvedValue([]);
  restore.push(...[message, listing, paths, auto, picker, importing, sources].map((mock) => () => mock.mockRestore()));
  return { listing, picker, importing, sources };
}

function Library({ visible = true }: { visible?: boolean }) {
  const library = useSkillsLibrary();
  return h('div', null,
    h('button', { onClick: library.importFromAgent }, 'Import from Agent'),
    h('button', { onClick: library.importFolder }, 'Import Skills'),
    h('button', { onClick: library.importZip }, 'Import .zip'),
    h(SkillsLibraryPanel, { library, visible })
  );
}
function page(visible = true) {
  return h(I18nextProvider, { i18n: locale }, h(MemoryRouter, null, h(Library, { visible })));
}

describe('Skills library import actions', () => {
  test('keeps folder and zip selection distinct and refreshes imported skills', async () => {
    const { listing, picker, importing } = prepareMocks();
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    picker.mockResolvedValueOnce(['/picked/folder']);
    listing.mockResolvedValueOnce([skill('from-folder')]);
    fireEvent.click(view.getByRole('button', { name: 'Import Skills' }));
    await view.findByTestId('skill-card-from-folder');
    expect(picker).toHaveBeenNthCalledWith(1, { properties: ['openDirectory'] });
    expect(importing).toHaveBeenNthCalledWith(1, { skill_path: '/picked/folder' });

    picker.mockResolvedValueOnce(['/picked/archive.zip']);
    listing.mockResolvedValueOnce([skill('from-zip')]);
    fireEvent.click(view.getByRole('button', { name: 'Import .zip' }));
    await view.findByTestId('skill-card-from-zip');
    expect(picker).toHaveBeenNthCalledWith(2, { properties: ['openFile'], filters: [{ name: 'Skill zip archives', extensions: ['zip'] }] });
    expect(importing).toHaveBeenNthCalledWith(2, { skill_path: '/picked/archive.zip' });
  });

  test('opens Agent import with external source detection', async () => {
    const { sources } = prepareMocks();
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Import from Agent' }));
    await waitFor(() => expect(sources).toHaveBeenCalledTimes(1));
    expect(view.getByTestId('agent-skill-import-drawer')).toBeTruthy();
  });

  test('does not import or reload when the picker is cancelled', async () => {
    const { picker, importing, listing } = prepareMocks();
    picker.mockResolvedValueOnce(undefined);
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Import Skills' }));
    await act(async () => {});
    expect(importing).not.toHaveBeenCalled();
    expect(listing).toHaveBeenCalledTimes(1);
  });

  test('keeps canonical skill preview, exact source and path available from details', async () => {
    prepareMocks();
    const source = '---\nname: installed\n---\n\n# Canonical instructions\n\nPreserve this content.';
    const reading = spyOn(ipcBridge.fs.readFile, 'invoke').mockResolvedValue(source);
    restore.push(() => reading.mockRestore());
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Skill Details: installed' }));

    await waitFor(() => expect(view.getByTestId('skill-detail-content').querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('Canonical instructions'));
    expect(reading).toHaveBeenCalledWith({ path: '/skills/installed/SKILL.md' });
    expect(view.getByText('/skills/installed/SKILL.md')).toBeTruthy();
    fireEvent.click(view.getByRole('tab', { name: 'Source' }));
    expect(view.getByTestId('skill-detail-source').textContent).toBe(source);
  });

  test('requires explicit confirmation before deleting a custom skill and refreshes after deletion', async () => {
    const { listing } = prepareMocks();
    const removing = spyOn(ipcBridge.fs.deleteSkill, 'invoke').mockResolvedValue(undefined);
    restore.push(() => removing.mockRestore());
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Delete installed' }));
    expect(removing).not.toHaveBeenCalled();
    const confirmation = await view.findByRole('dialog');
    expect(confirmation.textContent).toContain('installed');
    listing.mockResolvedValueOnce([]);
    await act(async () => { fireEvent.click(within(confirmation).getByRole('button', { name: 'Delete' })); });
    expect(removing).toHaveBeenCalledWith({ skill_name: 'installed' });
    expect(view.queryByTestId('skill-card-installed')).toBeNull();
    expect(view.getByTestId('skills-library-empty')).toBeTruthy();
  });

  test('lets an accepted import complete without restarting the library after navigation away', async () => {
    const { picker, importing, listing } = prepareMocks();
    picker.mockResolvedValueOnce(['/picked/folder']);
    let completeImport!: (result: { skill_name: string }) => void;
    importing.mockImplementationOnce(() => new Promise((resolve) => { completeImport = resolve; }));
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Import Skills' }));
    await waitFor(() => expect(importing).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(async () => { completeImport({ skill_name: 'after-navigation' }); });
    expect(listing).toHaveBeenCalledTimes(1);
  });

  test('hides an open skill drawer while another category is visible and keeps its selection', async () => {
    prepareMocks();
    const reading = spyOn(ipcBridge.fs.readFile, 'invoke').mockResolvedValue('# Skill instructions');
    restore.push(() => reading.mockRestore());
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    await act(async () => { fireEvent.click(view.getByRole('button', { name: 'Skill Details: installed' })); });
    const drawer = await view.findByTestId('skill-detail-drawer');
    await act(async () => view.rerender(page(false)));
    expect(drawer.classList.contains('arco-drawer-wrapper-hide')).toBe(true);
    await act(async () => view.rerender(page(true)));
    expect(drawer.classList.contains('arco-drawer-wrapper-hide')).toBe(false);
    expect(view.getByText('/skills/installed/SKILL.md')).toBeTruthy();
  });

  test('hides and restores deletion confirmation with its owning skills panel', async () => {
    prepareMocks();
    const removing = spyOn(ipcBridge.fs.deleteSkill, 'invoke').mockResolvedValue(undefined);
    restore.push(() => removing.mockRestore());
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Delete installed' }));
    await view.findByRole('dialog');
    await act(async () => view.rerender(page(false)));
    await waitFor(() => expect(view.queryByRole('dialog') === null).toBe(true));
    expect(removing).not.toHaveBeenCalled();
    await act(async () => view.rerender(page(true)));
    const confirmation = await view.findByRole('dialog');
    fireEvent.click(within(confirmation).getByRole('button', { name: 'Cancel' }));
    expect(removing).not.toHaveBeenCalled();
  });

  test('keeps a failed deletion retryable inside its confirmation', async () => {
    const { listing } = prepareMocks();
    const removing = spyOn(ipcBridge.fs.deleteSkill, 'invoke')
      .mockRejectedValueOnce(new Error('delete failed'))
      .mockResolvedValueOnce(undefined);
    const logging = spyOn(console, 'error').mockImplementation(() => {});
    restore.push(() => removing.mockRestore(), () => logging.mockRestore());
    const view = render(page());
    await view.findByTestId('skill-card-installed');
    fireEvent.click(view.getByRole('button', { name: 'Delete installed' }));
    const confirmation = await view.findByRole('dialog');
    await act(async () => { fireEvent.click(within(confirmation).getByRole('button', { name: 'Delete' })); });
    expect(view.getByTestId('skill-card-installed')).toBeTruthy();
    expect(view.queryByRole('dialog') !== null).toBe(true);
    listing.mockResolvedValueOnce([]);
    await act(async () => { fireEvent.click(within(confirmation).getByRole('button', { name: 'Delete' })); });
    expect(removing).toHaveBeenCalledTimes(2);
    expect(view.getByTestId('skills-library-empty')).toBeTruthy();
  });
});
