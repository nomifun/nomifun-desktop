/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { ICompanionProfile, ICompanionStatus } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
import zhNomi from '@/renderer/services/i18n/locales/zh-CN/nomi.json';
import CompanionAppearanceQuickSettings from './CompanionAppearanceQuickSettings';
import type { CompanionHandle } from './types';

const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: {
  nomi: zhNomi, agentSettings: { actions: { retry: '重试' } },
} } } });
const runtime = window as Window & { isTauri?: boolean };
const initialRuntime = runtime.isTauri;
afterEach(() => {
  cleanup();
  if (initialRuntime === undefined) delete runtime.isTauri;
  else runtime.isTauri = initialRuntime;
});

const companionId = parseCompanionId('019f0000-0000-7000-8000-000000000001');
const profile = { companion_id: companionId, name: '天天', mood_override: null,
  appearance: { companion_enabled: false } } as ICompanionProfile;
const status = { companion_id: companionId, mood: 'content', automatic_mood: 'content', xp: 0, level: 1,
  memories_active: 0, memories_archived: 0, model_configured: false, collect_any_enabled: false } satisfies ICompanionStatus;
const makeCompanion = (patchCompanion: CompanionHandle['patchCompanion']): CompanionHandle => ({
  profile, status, loading: false, patchCompanion, refresh: async () => {}, refreshStatus: async () => {},
});
const renderSettings = (companion: CompanionHandle, onSavingChange?: (saving: boolean) => void) =>
  render(<I18nextProvider i18n={i18n}><CompanionAppearanceQuickSettings companion={companion} onSavingChange={onSavingChange} /></I18nextProvider>);

test('failed manual mood keeps the previous value and retries the same patch', async () => {
  const patch = mock<CompanionHandle['patchCompanion']>()
    .mockRejectedValueOnce(new Error('offline'))
    .mockResolvedValueOnce({ ...profile, mood_override: 'happy' });
  const view = renderSettings(makeCompanion(patch));
  fireEvent.click(view.getByRole('combobox', { name: '状态' }));
  fireEvent.click(view.getByRole('option', { name: '开心' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('已保留原值'));
  expect(view.getByRole('combobox', { name: '状态' }).textContent).toContain('自动（平静）');
  expect(patch.mock.calls[0]).toEqual([{ mood_override: 'happy' }, { optimistic: false }]);
  fireEvent.click(view.getByRole('button', { name: '重试' }));
  await waitFor(() => expect(view.queryByRole('alert')).toBeNull());
  expect(patch.mock.calls[1]).toEqual(patch.mock.calls[0]);
});

test('returning to automatic writes null and labels the latest learned emotion', async () => {
  const patch = mock<CompanionHandle['patchCompanion']>().mockResolvedValue(profile);
  const view = renderSettings({ ...makeCompanion(patch), profile: { ...profile, mood_override: 'happy' },
    status: { ...status, mood: 'happy', automatic_mood: 'sleepy' } });
  expect(view.getByText('手动保持，切回“自动”时恢复')).toBeTruthy();
  fireEvent.click(view.getByRole('combobox', { name: '状态' }));
  fireEvent.click(view.getByRole('option', { name: '自动（犯困）' }));
  await waitFor(() => expect(patch).toHaveBeenCalledWith({ mood_override: null }, { optimistic: false }));
});

test('desktop display is unavailable in WebUI while mood remains editable', () => {
  delete runtime.isTauri;
  const patch = mock<CompanionHandle['patchCompanion']>().mockResolvedValue(profile);
  const view = renderSettings(makeCompanion(patch));
  expect(view.getByRole('combobox', { name: '桌面显示' }).getAttribute('aria-disabled')).toBe('true');
  expect(view.getByRole('combobox', { name: '状态' }).getAttribute('aria-disabled')).not.toBe('true');
  expect(view.getByText('请在桌面端设置桌面形象')).toBeTruthy();
  expect(patch).not.toHaveBeenCalled();
});

test('desktop save retains its committed value, blocks competing edits and releases the parent after unmount', async () => {
  runtime.isTauri = true;
  let finish!: (saved: ICompanionProfile) => void;
  const patch = mock<CompanionHandle['patchCompanion']>(() => new Promise((resolve) => { finish = resolve; }));
  const onSavingChange = mock();
  const view = renderSettings(makeCompanion(patch), onSavingChange);
  fireEvent.click(view.getByRole('combobox', { name: '桌面显示' }));
  fireEvent.click(view.getByRole('option', { name: '已显示' }));
  await waitFor(() => expect(patch).toHaveBeenCalledWith({ appearance: { companion_enabled: true } }, { optimistic: false }));
  expect(view.getByRole('combobox', { name: '桌面显示' }).textContent).toContain('已隐藏');
  expect(view.getByRole('combobox', { name: '状态' }).getAttribute('aria-disabled')).toBe('true');
  expect(onSavingChange).toHaveBeenLastCalledWith(true);
  view.unmount();
  await act(async () => { finish({ ...profile, appearance: { ...profile.appearance, companion_enabled: true } }); });
  expect(onSavingChange).toHaveBeenLastCalledWith(false);
});
