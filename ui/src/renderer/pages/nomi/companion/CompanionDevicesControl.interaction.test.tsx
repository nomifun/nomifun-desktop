import { afterEach, expect, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { robot, conversation } from '@/common/adapter/ipcBridge';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import CompanionDevicesControl from './CompanionDevicesControl';

const id = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000002');
const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.robot.devices': 'Devices', 'nomi.robot.playLastReply': 'Play latest reply',
  'nomi.robot.playbackPermissionHint': 'Allow desktop playback first', 'nomi.robot.chooseDevice': 'Choose device',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('multiple devices need an explicit target and playback respects that device permission', async () => {
  const list = spyOn(robot.list, 'invoke').mockResolvedValue(['Desk', 'Room'].map((name, index) => ({
    robot_id: `device-${index}`, name, companion_id: id, board: '', firmware_version: '', last_seen: null, created_at: '',
    permissions: { vision: false, motion: false, display: true, device_tools: false, continuous_vision: false, proactive_speech: index === 0 },
    supported_permissions: ['proactive_speech'],
  })));
  const statuses = spyOn(robot.statuses, 'invoke').mockResolvedValue([0, 1].map((index) => ({ robot_id: `device-${index}`, companion_id: id, phase: 'idle', changed_at: 1 })));
  const statusEvents = spyOn(robot.onStatus, 'on').mockImplementation(() => () => {});
  const reconnect = spyOn(conversation.reconnected, 'on').mockImplementation(() => () => {});
  const speak = spyOn(robot.speak, 'invoke').mockResolvedValue({ accepted: true });
  for (const spy of [list, statuses, statusEvents, reconnect, speak]) restores.push(() => spy.mockRestore());
  const patches: unknown[] = [];
  function View() {
    const [profile, setProfile] = useState({ companion_id: id, control_robot_id: null as string | null });
    return <CompanionDevicesControl conversationId={conversationId} companion={{ profile, patchCompanion: async (patch: any) => {
      patches.push(patch); setProfile((previous) => ({ ...previous, ...patch }));
    } } as any} />;
  }
  const view = render(<MemoryRouter><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}>
    <I18nextProvider i18n={i18n}><View /></I18nextProvider>
  </SWRConfig></MemoryRouter>);
  fireEvent.click(await view.findByRole('button', { name: 'Devices' }));
  expect((view.getByRole('button', { name: 'Play latest reply' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('radio', { name: 'Room' }));
  await waitFor(() => expect(patches).toContainEqual({ control_robot_id: 'device-1' }));
  expect(view.getByText('Allow desktop playback first')).toBeTruthy();
  expect((view.getByRole('button', { name: 'Play latest reply' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('radio', { name: 'Desk' }));
  await waitFor(() => expect((view.getByRole('button', { name: 'Play latest reply' }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(view.getByRole('button', { name: 'Play latest reply' }));
  await waitFor(() => expect(speak).toHaveBeenCalledWith({ robot_id: 'device-0', conversation_id: conversationId }));
});
