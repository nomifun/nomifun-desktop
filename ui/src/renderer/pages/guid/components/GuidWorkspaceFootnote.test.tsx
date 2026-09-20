/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { addRecentWorkspace } from '@/renderer/components/workspace';
import { ipcBridge } from '@/common';
import {
  addProjectWorkpath,
  getProjectWorkpaths,
  removeProjectWorkpath,
} from '@/renderer/pages/conversation/SessionList/utils/projectWorkpaths';
import GuidWorkspaceFootnote from './GuidWorkspaceFootnote';

const i18n = createInstance();
beforeAll(async () => {
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
});
afterEach(() => {
  cleanup();
  localStorage.clear();
});

const renderSelector = (workspaceDir = '', onClearWorkspace = mock(() => {})) => render(
  <I18nextProvider i18n={i18n}>
    <GuidWorkspaceFootnote
      workspaceDir={workspaceDir}
      onSelectWorkspace={() => {}}
      onClearWorkspace={onClearWorkspace}
    />
  </I18nextProvider>
);

describe('Guid project selector registry', () => {
  test('shows sidebar projects instead of the unrelated recent-folder cache', () => {
    addRecentWorkspace('/projects/stale-recent');
    addProjectWorkpath('/projects/sidebar-project');
    const view = renderSelector();

    fireEvent.click(view.getByTestId('workspace-selector-btn'));

    expect(view.getByText('/projects/sidebar-project')).toBeTruthy();
    expect(view.queryByText('/projects/stale-recent')).toBeNull();
  });

  test('removing a sidebar project removes it from the open selector', () => {
    addProjectWorkpath('/projects/remove-me');
    const view = renderSelector();
    fireEvent.click(view.getByTestId('workspace-selector-btn'));
    expect(view.getByText('/projects/remove-me')).toBeTruthy();

    act(() => removeProjectWorkpath('/projects/remove-me'));

    expect(view.queryByText('/projects/remove-me')).toBeNull();
  });

  test('removing the currently staged project clears the stale draft selection', () => {
    const clear = mock(() => {});
    addProjectWorkpath('/projects/current');
    renderSelector('/projects/current', clear);

    act(() => removeProjectWorkpath('/projects/current'));

    expect(clear).toHaveBeenCalledTimes(1);
  });

  test('choosing a new project from the conversation selector registers it in the sidebar source', async () => {
    const selected = mock(() => {});
    const dialog = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['C:\\projects\\new-project']);
    const view = render(
      <I18nextProvider i18n={i18n}>
        <GuidWorkspaceFootnote
          workspaceDir=''
          onSelectWorkspace={selected}
          onClearWorkspace={() => {}}
        />
      </I18nextProvider>
    );

    await act(async () => {
      fireEvent.click(view.getByTestId('workspace-selector-btn'));
      await Promise.resolve();
    });

    expect(getProjectWorkpaths()).toEqual(['C:/projects/new-project']);
    expect(selected).toHaveBeenCalledWith('C:/projects/new-project');
    dialog.mockRestore();
  });
});
