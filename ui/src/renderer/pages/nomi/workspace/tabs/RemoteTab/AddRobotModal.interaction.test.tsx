import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { agentPlatform, robot } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
import type { ProductAgentOptions, ProductAgentSelectionResult } from '@/common/types/agentPlatform';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import AddRobotModal from './AddRobotModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.robot.agentBeforeClaim': 'Choose an Agent after pairing',
  'nomi.robot.codePlaceholder': 'Activation code',
  'nomi.robot.claim': 'Pair',
  'nomi.robot.finishSetup': 'Finish setup',
  'agentSettings.template.robot.default.name': 'Robot',
  'agentSettings.template.chat.minimal.name': 'Minimal',
  'agentSettings.productBinding.chooseModelLater': 'Choose a model later',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('pairing leads to device Agent settings without requiring a model or changing the companion Agent', async () => {
  const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
  const deviceId = 'aa:bb:cc:dd:ee:ff';
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', fontScale: 1, colorScheme: 'default',
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  const endpoints = spyOn(robot.endpoints, 'invoke').mockResolvedValue({ ota_urls: [], lan_enabled: true });
  const claim = spyOn(robot.claim, 'invoke').mockResolvedValue({
    robot_id: deviceId, companion_id: companionId, name: 'Robot', board: '',
    firmware_version: '', last_seen: null, created_at: '',
  });
  let state: ProductAgentOptions = {
    selection: { kind: 'template', template_key: 'robot.default' }, needs_model: true,
    options: ['robot.default', 'chat.minimal'].map((key) => ({
      selection: { kind: 'template', template_key: key as 'robot.default' | 'chat.minimal' },
      display_name: '', available: true, reason: null,
    })),
  };
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke').mockImplementation(async () => state);
  let finishSave!: (result: ProductAgentSelectionResult) => void;
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke').mockImplementation(() => new Promise((resolve) => { finishSave = resolve; }));
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [themeSpy, endpoints, claim, options, save, toast]) restores.push(() => spy.mockRestore());
  const onClaimed = mock(() => {});
  const onCancel = mock(() => {});
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}>
    <I18nextProvider i18n={i18n}>
      <AddRobotModal visible companionId={companionId} companionName='Nomi' onClaimed={onClaimed} onCancel={onCancel} />
    </I18nextProvider>
  </SWRConfig>);
  expect(view.getByText('Choose an Agent after pairing')).toBeTruthy();
  expect(options).not.toHaveBeenCalled();
  fireEvent.change(view.getByPlaceholderText('Activation code'), { target: { value: '123456' } });
  fireEvent.click(view.getByRole('button', { name: 'Pair' }));
  await waitFor(() => expect(view.getByText('Choose a model later')).toBeTruthy());
  expect(onClaimed).toHaveBeenCalledTimes(1);
  expect(onCancel).not.toHaveBeenCalled();
  expect(view.queryByRole('button', { name: 'Pair' })).toBeNull();
  expect(options.mock.calls[0][0]).toEqual({ target_kind: 'robot', target_id: deviceId, model: undefined });
  fireEvent.click(view.baseElement.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText('Minimal'));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).toEqual({
    target_kind: 'robot', target_id: deviceId,
    request: { selection: { kind: 'template', template_key: 'chat.minimal' } },
  });
  expect((view.getByRole('button', { name: 'Finish setup' }) as HTMLButtonElement).disabled).toBe(true);
  state = { ...state, selection: { kind: 'template', template_key: 'chat.minimal' } };
  await act(async () => { finishSave({ selection: state.selection, needs_model: true }); });
  await waitFor(() => expect((view.getByRole('button', { name: 'Finish setup' }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(view.getByRole('button', { name: 'Finish setup' }));
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(claim).toHaveBeenCalledTimes(1);
});
