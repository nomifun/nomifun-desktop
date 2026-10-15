import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { robot as robotApi, type IApiRobot, type IApiRobotCapabilities } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import enNomi from '@/renderer/services/i18n/locales/en-US/nomi.json';
import RobotCapabilitiesModal from './RobotCapabilitiesModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { nomi: enNomi, common: { close: 'Close' } } } } });
const copy = enNomi.robot.capabilities;
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

const robot: IApiRobot = {
  robot_id: 'generic:robot:one', name: 'Robot', companion_id: null, board: 'arm-controller',
  firmware_version: '1.0', capability_revision: 4, last_seen: null, created_at: '',
  permissions: { vision: false, motion: false, display: false, device_tools: false, proactive_speech: false, continuous_vision: false },
  supported_permissions: ['motion'],
};
function response(connected = true): IApiRobotCapabilities {
  return { robot_id: robot.robot_id, connected, revision: 4,
    profile: { tool_overrides: {}, presets: [] }, tools: [{
      device_name: 'vendor.arm.pose', exposed_name: 'robot_vendor_arm_pose',
      description: 'Move a configurable arm', effective_description: 'Move a configurable arm',
      action: 'motion', allowed_actions: ['motion'], enabled: true, default_arguments: {},
      input_schema: { type: 'object', properties: {
        degrees: { type: 'integer', minimum: -45, maximum: 45, description: 'Arm rotation' },
        mode: { type: 'string', enum: ['gentle', 'quick'] },
        active: { type: 'boolean' },
        metadata: { type: 'object' },
      }, required: ['degrees'] },
    }] };
}
function setup(data = response()) {
  const themeSpy = spyOn(theme, 'useThemeContext').mockReturnValue({ theme: 'light', fontScale: 1,
    colorScheme: 'default', setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {} });
  const get = spyOn(robotApi.getCapabilities, 'invoke').mockResolvedValue(data);
  const save = spyOn(robotApi.setCapabilities, 'invoke').mockImplementation(async (request) => ({
    ...data, revision: data.revision + 1, profile: request.profile,
  }));
  const refresh = spyOn(robotApi.refreshCapabilities, 'invoke').mockResolvedValue({ ...data, revision: 5 });
  const permissions = spyOn(robotApi.setPermissions, 'invoke');
  for (const spy of [themeSpy, get, save, refresh, permissions]) restores.push(() => spy.mockRestore());
  const onSaved = mock(() => {});
  const view = render(<I18nextProvider i18n={i18n}><RobotCapabilitiesModal robot={robot}
    onCancel={() => {}} onSaved={onSaved} /></I18nextProvider>);
  return { ...view, get, save, refresh, permissions, onSaved };
}

test('a generic device schema edits defaults and named presets without executing hardware or granting permissions', async () => {
  const view = setup();
  await view.findByText('vendor.arm.pose');
  fireEvent.click(view.getByText('vendor.arm.pose'));
  expect(view.getByLabelText(copy.action).querySelectorAll('option')).toHaveLength(2);
  fireEvent.change(view.getByLabelText(copy.description), { target: { value: 'Use for a small greeting' } });
  fireEvent.click(view.getAllByLabelText(copy.includeArgument)[0]!);
  fireEvent.change(view.getByLabelText('degrees *'), { target: { value: '15' } });
  fireEvent.click(view.getAllByLabelText(copy.includeArgument)[1]!);
  fireEvent.change(view.getByLabelText('mode'), { target: { value: '"quick"' } });
  fireEvent.click(view.getByRole('button', { name: copy.addPreset }));
  fireEvent.change(view.getByLabelText(copy.presetName), { target: { value: 'Wave hello' } });
  fireEvent.change(view.getByLabelText(copy.presetId), { target: { value: 'wave_hello' } });
  const include = view.getAllByLabelText(copy.includeArgument);
  fireEvent.click(include[4]!);
  const degrees = view.getAllByLabelText('degrees *');
  fireEvent.change(degrees[1]!, { target: { value: '25' } });
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await waitFor(() => expect(view.save).toHaveBeenCalledTimes(1));
  expect(view.save.mock.calls[0]?.[0]).toEqual({ robot_id: robot.robot_id, expected_revision: 4,
    profile: { tool_overrides: { 'vendor.arm.pose': { enabled: true, action: null,
      description: 'Use for a small greeting', default_arguments: { degrees: 15, mode: 'quick' } } },
    presets: [{ id: 'wave_hello', name: 'Wave hello', tool_name: 'vendor.arm.pose',
      description: '', arguments: { degrees: 25 }, enabled: true }] } });
  await view.findByText(copy.saved);
  expect(view.permissions).not.toHaveBeenCalled();
  expect(view.refresh).not.toHaveBeenCalled();
  expect(view.onSaved).toHaveBeenCalledTimes(1);
});

test('a revision conflict preserves the draft and requires reloading before retrying', async () => {
  const view = setup();
  view.save.mockRejectedValue(new BackendHttpError({ method: 'PUT', path: '/api/robots/x/capabilities',
    status: 409, body: { error: 'revision changed' } }));
  await view.findByText('vendor.arm.pose');
  fireEvent.change(view.getByLabelText(copy.description), { target: { value: 'Keep my draft' } });
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await view.findByText(copy.conflict);
  expect((view.getByLabelText(copy.description) as HTMLTextAreaElement).value).toBe('Keep my draft');
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.get).toHaveBeenCalledTimes(1);
});

test('a device going offline during save preserves the draft and can be retried after reconnecting', async () => {
  const view = setup();
  view.save.mockRejectedValueOnce(new BackendHttpError({ method: 'PUT', path: '/api/robots/x/capabilities',
    status: 409, body: { code: 'ROBOT_OFFLINE', error: 'Connect the device' } }));
  await view.findByText('vendor.arm.pose');
  fireEvent.change(view.getByLabelText(copy.description), { target: { value: 'Keep the offline draft' } });
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await waitFor(() => expect(view.getAllByText(copy.offline).length).toBeGreaterThan(0));
  expect((view.getByLabelText(copy.description) as HTMLTextAreaElement).value).toBe('Keep the offline draft');
  expect(view.queryByText(copy.conflict)).toBeNull();
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await waitFor(() => expect(view.save).toHaveBeenCalledTimes(2));
  expect(view.save.mock.calls[1]?.[0].profile.tool_overrides['vendor.arm.pose']?.description).toBe('Keep the offline draft');
  await view.findByText(copy.saved);
});

test('refresh cannot discard unsaved edits and reload is an explicit choice', async () => {
  const view = setup();
  await view.findByText('vendor.arm.pose');
  fireEvent.change(view.getByLabelText(copy.description), { target: { value: 'Unsaved behavior' } });
  expect((view.getByRole('button', { name: copy.refresh }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.getByText(copy.draftHint)).toBeTruthy();
  expect((view.getByRole('button', { name: copy.reload }) as HTMLButtonElement).disabled).toBe(false);
  expect(view.refresh).not.toHaveBeenCalled();
});

test('invalid JSON cannot silently save the previous valid parameters', async () => {
  const view = setup();
  await view.findByText('vendor.arm.pose');
  fireEvent.click(view.getAllByLabelText(copy.includeArgument)[3]!);
  fireEvent.change(view.getByLabelText('metadata'), { target: { value: '{ invalid' } });
  expect(view.getByText(copy.jsonInvalid)).toBeTruthy();
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(view.getByLabelText(copy.description), { target: { value: 'Other field edit' } });
  expect((view.getByLabelText('metadata') as HTMLTextAreaElement).value).toBe('{ invalid');
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(view.getByLabelText('metadata'), { target: { value: '{"tag":"greeting"}' } });
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await waitFor(() => expect(view.save).toHaveBeenCalledTimes(1));
  expect(view.save.mock.calls[0]?.[0].profile.tool_overrides['vendor.arm.pose']?.default_arguments)
    .toEqual({ metadata: { tag: 'greeting' } });
});

test('offline devices expose cached capabilities with editing, saving and refresh disabled', async () => {
  const data = response(false);
  data.profile.tool_overrides['vendor.arm.pose'] = { enabled: false, action: null, description: 'Saved instructions', default_arguments: { degrees: 20 } };
  const view = setup(data);
  await view.findByText(copy.offline);
  expect((view.getByLabelText(copy.description) as HTMLTextAreaElement).value).toBe('Saved instructions');
  expect(view.getByLabelText(copy.description).closest('fieldset')?.disabled).toBe(true);
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByRole('button', { name: copy.refresh }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.save).not.toHaveBeenCalled();
});

test('restoring a tool removes only its override and preserves saved presets', async () => {
  const data = response();
  data.profile.tool_overrides['vendor.arm.pose'] = { enabled: false, action: 'motion', description: 'Old', default_arguments: { degrees: 20 } };
  data.profile.presets = [{ id: 'greeting', name: 'Greeting', tool_name: 'vendor.arm.pose', description: '', arguments: { degrees: 5 }, enabled: true }];
  const view = setup(data);
  await view.findByLabelText(copy.description);
  fireEvent.click(view.getByRole('button', { name: copy.restoreDefault }));
  fireEvent.click(view.getByRole('button', { name: copy.save }));
  await waitFor(() => expect(view.save).toHaveBeenCalledTimes(1));
  expect(view.save.mock.calls[0]?.[0].profile).toEqual({ tool_overrides: {}, presets: data.profile.presets });
});

test('removing another preset preserves invalid edits on the remaining preset', async () => {
  const data = response();
  data.profile.presets = ['one', 'two'].map((id) => ({ id, name: id, tool_name: 'vendor.arm.pose',
    description: '', arguments: { degrees: 5, metadata: {} }, enabled: true }));
  const view = setup(data);
  await view.findByLabelText(copy.description);
  const fields = view.getAllByLabelText('metadata');
  fireEvent.change(fields[1]!, { target: { value: '{ unsaved' } });
  expect((fields[1] as HTMLTextAreaElement).value).toBe('{ unsaved');
  const remainingId = fields[1]!.id;
  fireEvent.click(view.getAllByRole('button', { name: copy.removePreset })[0]!);
  expect(view.getByLabelText('metadata').id).toBe(remainingId);
  expect((view.getByLabelText('metadata') as HTMLTextAreaElement).value).toBe('{ unsaved');
  expect((view.getByRole('button', { name: copy.save }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.save).not.toHaveBeenCalled();
});

test('an explicit clean refresh reloads discovered capabilities and signals the conversation resource list', async () => {
  const view = setup();
  await view.findByText('vendor.arm.pose');
  fireEvent.click(view.getByRole('button', { name: copy.refresh }));
  await waitFor(() => expect(view.onSaved).toHaveBeenCalledTimes(1));
  expect(view.refresh).toHaveBeenCalledWith({ robot_id: robot.robot_id });
  expect(view.save).not.toHaveBeenCalled();
  expect(view.permissions).not.toHaveBeenCalled();
  expect(view.getByText('Capability revision 5')).toBeTruthy();
});
