import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { agentPlatform, robot } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import AddRobotModal from './AddRobotModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.robot.inheritHint': 'Uses {{companionName}} conversation and Agent settings',
  'nomi.robot.codePlaceholder': 'Activation code',
  'nomi.robot.claim': 'Pair',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('pairing completes with the companion identity without creating a device Agent selection', async () => {
  const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', fontScale: 1, colorScheme: 'default',
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  const endpoints = spyOn(robot.endpoints, 'invoke').mockResolvedValue({ ota_urls: [], lan_enabled: true });
  const claim = spyOn(robot.claim, 'invoke').mockResolvedValue({
    robot_id: 'aa:bb:cc:dd:ee:ff', companion_id: companionId, name: 'Robot', board: '',
    firmware_version: '', last_seen: null, created_at: '',
    supported_permissions: ['proactive_speech'],
    permissions: { vision: false, motion: false, display: true, device_tools: false,
      proactive_speech: false, continuous_vision: false },
  });
  const options = spyOn(agentPlatform.productBindingOptions, 'invoke');
  const save = spyOn(agentPlatform.selectProductBinding, 'invoke');
  const toast = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [themeSpy, endpoints, claim, options, save, toast]) restores.push(() => spy.mockRestore());
  const onClaimed = mock(() => {});
  const onCancel = mock(() => {});
  const view = render(<I18nextProvider i18n={i18n}>
    <AddRobotModal visible companionId={companionId} companionName='Nomi' onClaimed={onClaimed} onCancel={onCancel} />
  </I18nextProvider>);
  expect(view.getByText('Uses Nomi conversation and Agent settings')).toBeTruthy();
  fireEvent.change(view.getByPlaceholderText('Activation code'), { target: { value: '123456' } });
  fireEvent.click(view.getByRole('button', { name: 'Pair' }));
  await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1));
  expect(onClaimed).toHaveBeenCalledTimes(1);
  expect(claim).toHaveBeenCalledWith({ code: '123456', companion_id: companionId });
  expect(options).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});
