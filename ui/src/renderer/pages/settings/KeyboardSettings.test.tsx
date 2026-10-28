import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { act } from 'react';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { SystemPermissionState, SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import { keyboard, type KeyboardActionRequest, type KeyboardSettings as Settings, type KeyboardState } from '@/common/keyboard';
import en from '@/renderer/services/i18n/locales/en-US/settings.json';
import KeyboardSettings from './KeyboardSettings';

const original = { ...keyboard };
let permissionState: SystemPermissionState;
let permissionRequests: string[];
let openedPermissions: string[];
let permissionMocks: Array<{ mockRestore: () => void }>;
const permissionStatus = (): SystemPermissionStatus => ({ platform: 'macos', app_label: 'NomiFun', permissions: [{ kind: 'microphone', state: permissionState, can_request: permissionState === 'not_determined', can_open_settings: true, requires_restart_after_grant: false, capabilities: ['voice_input'] }] });
const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; };
const idleMicrophone = { id: null, deviceId: null, state: 'idle' as const, callbackObserved: false, samples: 0, nonzero: 0, peak: 0, rms: 0, elapsedMs: 0, stopped: true, detail: null };
const emit = (patch: Partial<KeyboardState>) => act(() => { current = { ...current, ...patch, sequence: current.sequence + 1 }; listener?.(current); });
let current: KeyboardState;
let actions: KeyboardActionRequest[];
let updates: Settings[];
let listener: ((state: KeyboardState) => void) | undefined;
beforeEach(() => {
  permissionState = 'granted'; permissionRequests = []; openedPermissions = [];
  permissionMocks = [
    spyOn(ipcBridge.systemPermissions.get, 'invoke').mockImplementation(async () => permissionStatus()),
    spyOn(ipcBridge.systemPermissions.request, 'invoke').mockImplementation(async ({ kind }) => { permissionRequests.push(kind); permissionState = 'granted'; return permissionStatus(); }),
    spyOn(ipcBridge.systemPermissions.openSettings, 'invoke').mockImplementation(async ({ kind }) => { openedPermissions.push(kind); }),
  ];
  current = {
    settings: { enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true, audioDeviceId: null, asrMode: 'streaming',
      asrModel: { providerId: 'provider-1', model: 'streaming-model' }, textModel: null,
      language: '', dictionary: [], beautifyInstruction: '', alwaysOnTop: false, subtitleFontSize: 14 },
    sequence: 0, cancelGeneration: 0, documentId: 'doc-1', revision: 2, operationId: null, editSessionId: null, dictationSelection: null, phase: 'idle', text: 'Ready text', originalText: 'Ready text',
    partial: '', candidate: '', visible: false, pendingAction: null, error: null, notice: null,
    audioState: { state: 'stopped', detail: null },
    capturePreparing: false, microphoneTest: { ...idleMicrophone },
    device: { connected: false, id: null, port: null, firmware: null,
      policyState: 'applied', captureState: 'stopped', detail: null },
    service: { running: false, ready: false, detail: null },
  };
  actions = []; updates = []; listener = undefined;
  keyboard.available = () => true;
  keyboard.getState = async () => current;
  const callbacks = new Set<(state: KeyboardState) => void>();
  listener = next => { for (const callback of callbacks) callback(next); };
  keyboard.listen = async callback => { callbacks.add(callback); return () => { callbacks.delete(callback); }; };
  keyboard.getCatalog = async () => ({
    asr: [{ providerId: 'provider-1', model: 'streaming-model', label: 'Streaming model', protocol: 'streaming' }],
    platformAsr: [{ providerId: 'provider-1', model: 'file-model', label: 'File recognition model', protocol: 'upload' }], text: [],
  });
  keyboard.getAudioDevices = async () => [{ id: 'microphone-1', label: 'Desk microphone', isDefault: false }];
  keyboard.action = async (request) => { actions.push(request); return current; };
  keyboard.updateSettings = async (settings) => { updates.push(settings); current = { ...current, sequence: current.sequence + 1, settings }; return current; };
});
afterEach(() => { cleanup(); permissionMocks.forEach(mock => mock.mockRestore()); Object.assign(keyboard, original); });
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings: { keyboard: en.keyboard } } } } });
const show = () => render(<SWRConfig value={{ provider: () => new Map() }}><I18nextProvider i18n={i18n}><MemoryRouter><KeyboardSettings /></MemoryRouter></I18nextProvider></SWRConfig>);

describe('system microphone keyboard settings', () => {
  test('the app-owned keyboard has no separate service startup or quit controls', async () => {
    show();
    await screen.findByRole('region', { name: 'Keyboard device' });
    expect(Boolean(screen.queryByRole('region', { name: 'Background device service' }))).toBe(false);
    expect(Boolean(screen.queryByRole('switch', { name: 'Start at user login' }))).toBe(false);
    expect(Boolean(screen.queryByRole('button', { name: 'Quit all services' }))).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Open subtitle board' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'open' }]));
  });

  test.each([
    { running: true, ready: true, label: 'Ready' },
    { running: true, ready: false, label: 'Preparing keyboard' },
    { running: false, ready: false, label: 'Not running' },
  ])('the device panel shows app-owned keyboard readiness: $label', async ({ running, ready, label }) => {
    current.service = { running, ready, detail: 'Keyboard runtime detail' };
    show();
    const device = within(await screen.findByRole('region', { name: 'Keyboard device' }));
    expect(device.getByText('Keyboard readiness')).toBeTruthy();
    expect(device.getByText(label)).toBeTruthy();
    expect(device.getByText('Keyboard runtime detail')).toBeTruthy();
  });

  test('the explicit microphone test works without a connected keyboard', async () => {
    show();
    const start = await screen.findByRole('button', { name: 'Test cloud transcription' });
    await waitFor(() => expect(start.hasAttribute('disabled')).toBe(false));
    fireEvent.click(start);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_start' }]));
  });

  test('connected keyboard shows its identity and firmware without calibration or onboard controls', async () => {
    current.device = { ...current.device, connected: true, id: 'NK-TEST-DEVICE', port: 'COM9', firmware: '0.3-screen.test',
      detail: '键盘采音已关闭；等待旧按键释放' };
    show();
    const device = within(await screen.findByRole('region', { name: 'Keyboard device' }));
    expect(device.getByText('Connected')).toBeTruthy();
    expect(device.getByText('NK-TEST-DEVICE · COM9')).toBeTruthy();
    expect(device.getByText('0.3-screen.test')).toBeTruthy();
    expect(device.getByText('Use four keys to control NomiFun. Recording uses your computer’s system microphone.')).toBeTruthy();
    expect(device.queryAllByRole('combobox')).toHaveLength(0);
    expect(device.queryAllByRole('button')).toHaveLength(0);
    expect(device.queryByText(/onboard|calibrat|NONE|waiting for.*key|等待旧按键|映射/i)).toBeNull();
    expect(screen.getByRole('switch', { name: 'K2 · Beautify full text' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'K3 · Cancel current recognition' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'K4 · Hold to speak' })).toBeTruthy();
  });

  test('disconnected keyboard retains its connection status and independent microphone selection', async () => {
    show();
    const device = within(await screen.findByRole('region', { name: 'Keyboard device' }));
    expect(device.getByText('Disconnected')).toBeTruthy();
    expect(device.getByText('Not read yet')).toBeTruthy();
    expect(screen.getByRole('combobox', { name: 'Recording device' })).toBeTruthy();
  });

  test('active system recording remains visible with its finish action', async () => {
    current.phase = 'recording'; current.audioState = { state: 'capturing', detail: 'Desk microphone' };
    show();
    await screen.findByText('Recording system microphone');
    expect(screen.getByText('Desk microphone')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Finish cloud test' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('a previous hardware recognition error does not become a settings failure', async () => {
    current.error = 'Recognition failed. Check the provider connection and try again.';
    show();
    const start = await screen.findByRole('button', { name: 'Test cloud transcription' });
    expect(within(screen.getByRole('region', { name: 'Recent keyboard error' })).getByText(current.error)).toBeTruthy();
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).queryByText(current.error)).toBeNull();
    expect(start.hasAttribute('disabled')).toBe(false);
    fireEvent.click(start);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_start' }]));
  });

  test('an explicitly requested microphone test reports its own later recognition failure', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, phase: 'recording', operationId: 'test-operation', audioState: { state: 'capturing', detail: null } };
      return current;
    };
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await screen.findByText('Recording system microphone');
    const failure = 'The requested test connection failed. Try again.';
    act(() => {
      current = { ...current, sequence: 2, phase: 'idle', operationId: null, error: failure, audioState: { state: 'stopped', detail: null } };
      listener?.(current);
    });
    await within(screen.getByRole('region', { name: 'Cloud transcription test' })).findByText(failure);
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(false);
    act(() => {
      current = { ...current, sequence: 3, error: 'Unrelated later hardware failure' };
      listener?.(current);
    });
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).queryByText('Unrelated later hardware failure')).toBeNull();
    expect(within(screen.getByRole('region', { name: 'Recent keyboard error' })).getByText('Unrelated later hardware failure')).toBeTruthy();
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).getByText(failure)).toBeTruthy();
  });

  test('a replaced operation cannot report its hardware failure as a microphone test result', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, capturePreparing: true, operationId: 'test-operation' };
      return current;
    };
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await waitFor(() => expect(actions).toHaveLength(1));
    act(() => {
      current = { ...current, sequence: 2, phase: 'recording', capturePreparing: false, operationId: 'hardware-operation', audioState: { state: 'capturing', detail: null } };
      listener?.(current);
    });
    await screen.findByText('Recording system microphone');
    act(() => {
      current = { ...current, sequence: 3, phase: 'idle', operationId: null, error: 'Later hardware failure', audioState: { state: 'stopped', detail: null } };
      listener?.(current);
    });
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).queryByText('Later hardware failure')).toBeNull();
    expect(within(screen.getByRole('region', { name: 'Recent keyboard error' })).getByText('Later hardware failure')).toBeTruthy();
  });

  test('a failed automatic setting keeps the edited switch and exposes one retry without dropping input', async () => {
    let attempts = 0;
    keyboard.updateSettings = async settings => { updates.push(settings); if (++attempts === 1) throw new Error('The keyboard settings could not be saved.'); current = { ...current, sequence: current.sequence + 1, settings }; return current; };
    show();
    const toggle = await screen.findByRole('switch', { name: 'K4 · Hold to speak' });
    fireEvent.click(toggle);
    await screen.findByText('The keyboard settings could not be saved.');
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    expect(current.settings.k4Enabled).toBe(true);
    expect(updates).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    expect(within(screen.getByRole('region', { name: 'Local microphone check' })).getByText('Settings have not been saved. Retry saving before testing.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Retry saving' }));
    await screen.findByText('Settings saved automatically');
    expect(updates).toHaveLength(2); expect(current.settings.k4Enabled).toBe(false);
  });

  test('system microphone stopping status is shown without an onboard capture row', async () => {
    current.device.captureState = 'stopping'; current.audioState.state = 'stopping';
    show();
    await screen.findByText('Stopping system recording');
    expect(screen.queryByText('Awaiting onboard stop')).toBeNull();
  });

  test('a pending microphone start can be stopped and cannot be started again', async () => {
    current.audioState = { state: 'starting', detail: 'Preparing recognition' };
    show();
    await screen.findByText('Starting system recording');
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    const stop = screen.getByRole('button', { name: 'Finish cloud test' });
    expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('waiting for the editor to save can be stopped before the microphone starts', async () => {
    current.capturePreparing = true;
    show();
    await screen.findByText('Not recording');
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    const stop = screen.getByRole('button', { name: 'Finish cloud test' });
    expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('a selected system microphone is saved as a device reference in the native settings', async () => {
    show();
    await screen.findByText('Not recording');
    const select = screen.getByRole('combobox', { name: 'Recording device' });
    fireEvent.click(select);
    fireEvent.click(await screen.findByText('Desk microphone'));
    await waitFor(() => expect(updates.length).toBe(1));
    expect(updates[0].audioDeviceId).toBe('microphone-1');
    expect(updates[0].asrModel).toEqual(current.settings.asrModel);
    expect(updates[0]).not.toHaveProperty('launchAtLogin');
  });

  test('switching to standard ASR filters models, clears an incompatible reference and saves the explicit mode', async () => {
    show();
    const mode = await screen.findByRole('combobox', { name: 'Recognition mode' });
    fireEvent.click(mode);
    fireEvent.click(await screen.findByText('Recognize after recording (standard ASR)'));
    await screen.findByText('Only configured cloud models that accept completed audio uploads are listed. This mode does not show live transcription.');
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0].asrMode).toBe('platform'); expect(updates[0].asrModel).toBeNull();
    fireEvent.click(screen.getByRole('combobox', { name: 'ASR model' }));
    const fileModel = await screen.findByText('File recognition model');
    expect(screen.queryByText('Streaming model')).toBeNull();
    fireEvent.click(fileModel);
    await waitFor(() => expect(updates).toHaveLength(2));
    expect(updates[1].asrMode).toBe('platform');
    expect(updates[1].asrModel).toEqual({ providerId: 'provider-1', model: 'file-model' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Test cloud transcription' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'test_start' }]));
  });

  test('switching back to streaming requires a streaming-capable model', async () => {
    current.settings.asrMode = 'platform';
    current.settings.asrModel = { providerId: 'provider-1', model: 'file-model' };
    show();
    fireEvent.click(await screen.findByRole('combobox', { name: 'Recognition mode' }));
    fireEvent.click(await screen.findByText('Live streaming recognition'));
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0].asrMode).toBe('streaming'); expect(updates[0].asrModel).toBeNull();
    fireEvent.click(screen.getByRole('combobox', { name: 'ASR model' }));
    const streamingModel = await screen.findByText('Streaming model');
    expect(screen.queryByText('File recognition model')).toBeNull();
    fireEvent.click(streamingModel);
    await waitFor(() => expect(updates).toHaveLength(2));
    expect(updates[1].asrMode).toBe('streaming');
    expect(updates[1].asrModel).toEqual({ providerId: 'provider-1', model: 'streaming-model' });
  });

  test('subtitle font loads without writes and a changed selection saves automatically', async () => {
    current.settings.subtitleFontSize = 20;
    show();
    const font = await screen.findByRole('combobox', { name: 'Subtitle font size' });
    expect(font.textContent).toContain('20px'); expect(updates).toHaveLength(0);
    expect(Boolean(screen.queryByRole('button', { name: 'Save settings' }))).toBe(false);
    expect(Boolean(screen.queryByRole('button', { name: 'Discard unsaved changes' }))).toBe(false);
    fireEvent.click(font); fireEvent.click(await screen.findByText('12px'));
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(current.settings.subtitleFontSize).toBe(12);
    await screen.findByText('Settings saved automatically');
  });

  test('subtitle font exposes only integer 12–32 choices and saves through the existing settings request', async () => {
    show();
    const font = await screen.findByRole('combobox', { name: 'Subtitle font size' });
    expect(font.textContent).toContain('14px');
    fireEvent.click(font);
    await screen.findByText('12px'); const maximum = await screen.findByText('32px');
    expect(screen.queryByText('11px')).toBeNull(); expect(screen.queryByText('33px')).toBeNull();
    expect(screen.queryByText('14.5px')).toBeNull();
    fireEvent.click(maximum);
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0].subtitleFontSize).toBe(32);
    expect(updates[0].asrModel).toEqual(current.settings.asrModel);
    expect(updates[0].audioDeviceId).toBe(current.settings.audioDeviceId);
    await screen.findByText('Settings saved automatically');
  });
  test('initial setup distinguishes hardware communication and unverified speech checks', async () => {
    current.device.connected = true; current.service = { running: true, ready: true, detail: null };
    show();
    const setup = within(await screen.findByRole('region', { name: 'First-use checks' }));
    expect(setup.getByText('Keyboard and K4 ready')).toBeTruthy();
    expect(setup.getByText('Local input not tested')).toBeTruthy();
    expect(setup.getByText('Cloud transcription not tested')).toBeTruthy();
    expect(setup.getByRole('button', { name: 'Check microphone' })).toBeTruthy();
    expect(actions).toHaveLength(0); expect(permissionRequests).toHaveLength(0); expect(updates).toHaveLength(0);
  });

  test.each(['not_determined', 'unknown', 'granted'] as const)('local test is available with %s permission and no ASR or keyboard', async permission => {
    permissionState = permission; current.settings.asrModel = null; current.settings.k4Enabled = false;
    show();
    const local = within(await screen.findByRole('region', { name: 'Local microphone check' }));
    const start = local.getByRole('button', { name: 'Test local microphone' });
    await waitFor(() => expect(start.hasAttribute('disabled')).toBe(false));
    fireEvent.click(start);
    await waitFor(() => expect(actions).toHaveLength(1));
    expect(actions[0].action).toBe('microphone_test_start'); expect(actions[0].audioDeviceId).toBeNull();
    expect(actions[0].testId).toBeTruthy(); expect(permissionRequests).toHaveLength(0); expect(updates).toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
  });

  test.each(['denied', 'restricted'] as const)('blocked %s permission has a visible reason and explicit system settings action', async permission => {
    permissionState = permission; show();
    const local = within(await screen.findByRole('region', { name: 'Local microphone check' }));
    await waitFor(() => expect(local.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(true));
    expect(local.getByText('Microphone access is blocked. Review system privacy settings.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Open microphone settings' }));
    await waitFor(() => expect(openedPermissions).toEqual(['microphone']));
    expect(permissionRequests).toHaveLength(0); expect(actions).toHaveLength(0);
  });

  test('an undetermined permission is only requested on the explicit action', async () => {
    permissionState = 'not_determined'; show();
    fireEvent.click(await screen.findByRole('button', { name: 'Allow microphone access' }));
    await waitFor(() => expect(permissionRequests).toEqual(['microphone']));
    expect(actions).toHaveLength(0);
  });

  test('a local microphone check waits for the selected input automatic-save acknowledgement', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.getAudioDevices = async () => [{ id: 'microphone-2', label: 'USB microphone', isDefault: false }];
    keyboard.updateSettings = async settings => { updates.push(settings); return pending.promise; };
    show();
    fireEvent.click(await screen.findByRole('combobox', { name: 'Recording device' }));
    fireEvent.click(await screen.findByText('USB microphone'));
    await waitFor(() => expect(updates).toHaveLength(1));
    const local = within(screen.getByRole('region', { name: 'Local microphone check' }));
    expect(local.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(true);
    expect(local.getByText('Wait for settings to finish saving.')).toBeTruthy();
    expect(actions).toHaveLength(0);
    await act(async () => { current = { ...current, sequence: 1, settings: updates[0] }; pending.resolve(current); });
    await waitFor(() => expect(local.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(false));
    fireEvent.click(local.getByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1)); expect(actions[0].audioDeviceId).toBe('microphone-2');
    expect(updates).toHaveLength(1);
  });

  test('an invalid input and mode-mismatched saved model explain cloud and local blockers', async () => {
    current.settings.audioDeviceId = 'gone'; current.settings.asrModel = { providerId: 'provider-1', model: 'file-model' };
    show();
    const local = within(await screen.findByRole('region', { name: 'Local microphone check' }));
    await waitFor(() => expect(local.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(true));
    expect(local.getByText('Choose an available input or refresh the device list.')).toBeTruthy();
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    expect(cloud.getByText('Choose an available ASR model for the selected recognition mode.')).toBeTruthy();
    expect(cloud.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
  });

  test('local result reports native callbacks, falling levels and zero-audio completion independently', async () => {
    keyboard.action = async request => { actions.push(request); return { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'starting', stopped: false } }; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1));
    const id = actions[0].testId!;
    emit({ microphoneTest: { ...idleMicrophone, id, state: 'listening', stopped: false, callbackObserved: true, samples: 1600, nonzero: 100, peak: 0.8, rms: 0.4 } });
    await screen.findByText('Native input callback received');
    expect(screen.getByRole('meter', { name: 'Microphone input level' }).getAttribute('value')).toBe('0.8');
    emit({ microphoneTest: { ...idleMicrophone, id, state: 'listening', stopped: false, callbackObserved: true, samples: 3200, peak: 0.1, rms: 0.05 } });
    await waitFor(() => expect(screen.getByRole('meter', { name: 'Microphone input level' }).getAttribute('value')).toBe('0.1'));
    emit({ microphoneTest: { ...idleMicrophone, id, state: 'completed', callbackObserved: true, samples: 80000, elapsedMs: 5000 } });
    await screen.findByText('Input received only silence. Check mute and input volume, then try again.');
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).getByText('Cloud transcription not tested')).toBeTruthy();
  });

  test('unmount precisely stops its pending request and repeats the stop after its late acknowledgement', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.action = async request => { actions.push(request); return request.action === 'microphone_test_start' ? pending.promise : current; };
    const view = show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1)); const id = actions[0].testId!;
    view.unmount();
    await waitFor(() => expect(actions.filter(action => action.action === 'microphone_test_stop' && action.testId === id)).toHaveLength(1));
    await act(async () => { pending.resolve({ ...current, sequence: 2, microphoneTest: { ...idleMicrophone, id, state: 'starting', stopped: false } }); });
    await waitFor(() => expect(actions.filter(action => action.action === 'microphone_test_stop' && action.testId === id)).toHaveLength(2));
    expect(actions.every(action => action.testId === id)).toBe(true);
  });

  test('another local test ID cannot become this page result or receive its stop', async () => {
    keyboard.action = async request => { actions.push(request); return { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'starting', stopped: false } }; };
    const view = show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1)); const id = actions[0].testId!;
    emit({ microphoneTest: { ...idleMicrophone, id: 'someone-else', state: 'failed', detail: 'Other local test failed' } });
    expect(screen.queryByText('Other local test failed')).toBeNull();
    view.unmount(); await waitFor(() => expect(actions.some(action => action.action === 'microphone_test_stop' && action.testId === id)).toBe(true));
    expect(actions.some(action => action.testId === 'someone-else')).toBe(false);
  });

  test('a stop clicked before local start acknowledgement survives a no-op stop response', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.action = async request => { actions.push(request); return request.action === 'microphone_test_start' ? pending.promise : current; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1)); const id = actions[0].testId!;
    fireEvent.click(screen.getByRole('button', { name: 'Stop local check' }));
    await screen.findByText('Stopping and waiting for the native input to close…');
    await act(async () => { pending.resolve({ ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id, state: 'starting', stopped: false } }); });
    await waitFor(() => expect(actions.filter(action => action.action === 'microphone_test_stop' && action.testId === id)).toHaveLength(2));
    expect(screen.queryByText('Opening native microphone input…')).toBeNull();
  });

  test('a cloud start acknowledgement cannot pass by replaying the existing subtitle text', async () => {
    keyboard.action = async request => { actions.push(request); return { ...current, sequence: 1 }; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    await waitFor(() => expect(actions).toHaveLength(1));
    expect(Boolean(cloud.queryByText('Cloud transcription returned text'))).toBe(false);
    expect(Boolean(cloud.queryByText('Ready text'))).toBe(false);
  });

  test('a successful local result is invalidated after a draft input change', async () => {
    keyboard.action = async request => { actions.push(request); return { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'completed', callbackObserved: true, samples: 80000, nonzero: 600 } }; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await within(screen.getByRole('region', { name: 'Local microphone check' })).findByText('Local microphone input verified');
    fireEvent.click(screen.getByRole('combobox', { name: 'Recording device' }));
    fireEvent.click(await screen.findByText('Desk microphone'));
    await waitFor(() => expect(screen.queryByText('Local microphone input verified')).toBeNull());
    expect(updates).toHaveLength(1);
  });

  test.each([
    { result: { state: 'completed' as const, samples: 0, callbackObserved: false, stopped: true, detail: null }, message: 'No input samples were received. Check the input device, system permission and audio service.' },
    { result: { state: 'failed' as const, samples: 0, callbackObserved: false, stopped: true, detail: 'Native input could not open' }, message: 'Native input could not open' },
  ])('a local native result remains scoped to the local check: $result.state', async ({ result, message }) => {
    keyboard.action = async request => { actions.push(request); return { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, ...result, id: request.testId! } }; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await within(screen.getByRole('region', { name: 'Local microphone check' })).findByText(message);
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).queryByText(message)).toBeNull();
  });

  test('an empty streaming catalogue offers an ordinary ASR choice without changing the saved mode', async () => {
    current.settings.asrModel = null;
    keyboard.getCatalog = async () => ({ asr: [], platformAsr: [{ providerId: 'provider-1', model: 'file-model', label: 'File recognition model', protocol: 'upload' }], text: [] });
    show();
    await screen.findByText('Configured standard ASR models are available. You can choose Recognize after recording (standard ASR), then select a model.');
    expect(current.settings.asrMode).toBe('streaming'); expect(current.settings.asrModel).toBeNull(); expect(updates).toHaveLength(0);
    expect(screen.getByRole('combobox', { name: 'Recognition mode' }).textContent).toContain('Live streaming recognition');
  });

  test('a rejected local start reply preserves stop ownership after a native listening event', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      return request.action === 'microphone_test_start' ? pending.promise
        : { ...current, sequence: 2, microphoneTest: { ...current.microphoneTest, state: 'cancelled', stopped: true } };
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await waitFor(() => expect(actions).toHaveLength(1));
    emit({ audioState: { state: 'capturing', detail: null }, microphoneTest: { ...idleMicrophone, id: actions[0].testId!, state: 'listening', stopped: false, callbackObserved: true, samples: 1600 } });
    await act(async () => { pending.reject(new Error('Start reply transport failed')); });
    await screen.findByText('Start reply transport failed');
    const start = screen.getByRole('button', { name: 'Test local microphone' });
    const stop = screen.getByRole('button', { name: 'Stop local check' });
    expect(start.hasAttribute('disabled')).toBe(true); expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions.some(action => action.action === 'microphone_test_stop' && action.testId === actions[0].testId)).toBe(true));
  });

  test('a rejected local stop keeps the exact owner retryable until native stop proof', async () => {
    let stops = 0;
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'microphone_test_start') return { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'listening', stopped: false, callbackObserved: true } };
      stops++;
      if (stops === 1) throw new Error('Stop reply transport failed');
      return { ...current, sequence: 2, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'cancelled', stopped: true } };
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await screen.findByText('Listening locally… Speak to check the input level.');
    fireEvent.click(screen.getByRole('button', { name: 'Stop local check' }));
    await screen.findByText('Stop reply transport failed');
    expect(screen.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: 'Stop local check' }).hasAttribute('disabled')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Stop local check' }));
    await screen.findByText('Local microphone check cancelled');
    expect(actions.filter(action => action.action === 'microphone_test_stop').map(action => action.testId)).toEqual([actions[0].testId!, actions[0].testId!]);
  });

  test('listening events after stop cannot undo the stop intent while waiting for native closure', async () => {
    const pendingStop = deferred<KeyboardState>();
    keyboard.action = async request => { actions.push(request); return request.action === 'microphone_test_start' ? { ...current, sequence: 1, microphoneTest: { ...idleMicrophone, id: request.testId!, state: 'listening', stopped: false } } : pendingStop.promise; };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await screen.findByText('Listening locally… Speak to check the input level.');
    fireEvent.click(screen.getByRole('button', { name: 'Stop local check' }));
    emit({ sequence: 2, microphoneTest: { ...idleMicrophone, id: actions[0].testId!, state: 'listening', stopped: false } });
    expect(Boolean(screen.queryByText('Listening locally… Speak to check the input level.'))).toBe(false);
    expect(screen.getByText('Stopping and waiting for the native input to close…')).toBeTruthy();
    await act(async () => { pendingStop.resolve({ ...current, sequence: 3, microphoneTest: { ...current.microphoneTest, state: 'cancelled', stopped: true } }); });
    await screen.findByText('Local microphone check cancelled');
  });

  test('a rejected admission recovers after precise stop proves this test has no owner', async () => {
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'microphone_test_start') throw new Error('Session is locked');
      return { ...current, sequence: 2, microphoneTest: { ...idleMicrophone, id: 'another-page', state: 'completed', stopped: true } };
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test local microphone' }));
    await screen.findByText('Session is locked');
    fireEvent.click(screen.getByRole('button', { name: 'Stop local check' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Test local microphone' }).hasAttribute('disabled')).toBe(false));
    expect(actions.map(action => action.action)).toEqual(['microphone_test_start', 'microphone_test_stop']);
    expect(actions[1].testId).toBe(actions[0].testId); expect(actions.some(action => action.testId === 'another-page')).toBe(false);
  });

  test('a cloud recording with no new final text cannot pass by retaining the existing document', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, phase: 'recording', operationId: 'test-operation', audioState: { state: 'capturing', detail: null } };
      return current;
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await screen.findByText('Recording system microphone');
    emit({ phase: 'idle', operationId: null, audioState: { state: 'stopped', detail: null } });
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    await cloud.findByText('The test ended without new transcription. Check the provider and speak clearly, then try again.');
    expect(Boolean(cloud.queryByText('Cloud transcription returned text'))).toBe(false);
    expect(Boolean(cloud.queryByText('Ready text'))).toBe(false);
  });

  test('a cancelled cloud recording reports cancellation while preserving the existing document', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, phase: 'recording', operationId: 'test-operation', audioState: { state: 'capturing', detail: null } };
      return current;
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await screen.findByText('Recording system microphone');
    emit({ phase: 'idle', operationId: null, cancelGeneration: 1, audioState: { state: 'stopped', detail: null } });
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    await cloud.findByText('Cloud transcription test cancelled');
    expect(Boolean(cloud.queryByText('Cloud transcription returned text'))).toBe(false);
    expect(Boolean(cloud.queryByText('Ready text'))).toBe(false);
  });

  test.each([
    { selection: null, text: 'Ready textNew words.', result: 'New words.' },
    { selection: { anchor: 0, focus: 10 }, text: 'Ready text', result: 'Ready text' },
  ])('a cloud result requires a new owned commit and shows its submitted text: $result', async ({ selection, text, result }) => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, phase: 'recording', operationId: 'test-operation', dictationSelection: selection, audioState: { state: 'capturing', detail: null } };
      return current;
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await screen.findByText('Recording system microphone');
    emit({ phase: 'idle', operationId: null, revision: 3, text, dictationSelection: null, audioState: { state: 'stopped', detail: null } });
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    await cloud.findByText('Cloud transcription returned text');
    expect(cloud.getByText(result)).toBeTruthy();
    if (selection === null) expect(Boolean(cloud.queryByText('Ready textNew words.'))).toBe(false);
  });

  test('a batched cancellation before a new hardware recording cannot replace the cloud test owner', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, capturePreparing: true, operationId: null, audioState: { state: 'starting', detail: null } };
      return current;
    };
    show(); fireEvent.click(await screen.findByRole('button', { name: 'Test cloud transcription' }));
    await screen.findByText('Starting system recording');
    act(() => {
      current = { ...current, sequence: 2, capturePreparing: false, cancelGeneration: 1, phase: 'idle', audioState: { state: 'stopped', detail: null } };
      listener?.(current);
      current = { ...current, sequence: 3, phase: 'recording', operationId: 'hardware-operation', audioState: { state: 'capturing', detail: null } };
      listener?.(current);
    });
    emit({ phase: 'idle', operationId: null, revision: 3, text: 'Ready textHardware words.', audioState: { state: 'stopped', detail: null } });
    const cloud = within(screen.getByRole('region', { name: 'Cloud transcription test' }));
    await cloud.findByText('Cloud transcription test cancelled');
    expect(Boolean(cloud.queryByText('Cloud transcription returned text'))).toBe(false);
    expect(Boolean(cloud.queryByText('Hardware words.'))).toBe(false);
  });

  test('a selected model saves automatically but readiness and cloud wait for its actual acknowledgement', async () => {
    current.settings.asrModel = null;
    const pending = deferred<KeyboardState>();
    keyboard.updateSettings = async settings => { updates.push(settings); return pending.promise; };
    show();
    const setup = within(await screen.findByRole('region', { name: 'First-use checks' }));
    fireEvent.click(screen.getByRole('combobox', { name: 'ASR model' })); fireEvent.click(await screen.findByText('Streaming model'));
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(Boolean(setup.queryByText('Saved ASR model matches the recognition mode'))).toBe(false);
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    expect(within(screen.getByRole('region', { name: 'Cloud transcription test' })).getByText('Wait for settings to finish saving.')).toBeTruthy();
    expect(Boolean(screen.queryByRole('button', { name: 'Save current settings' }))).toBe(false);
    await act(async () => { current = { ...current, sequence: 1, settings: updates[0] }; pending.resolve(current); });
    await setup.findByText('Saved ASR model matches the recognition mode');
    await screen.findByText('Settings saved automatically');
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(false);
  });

  test('text input is visible while queued, blurs into a save and keeps newer typing when its older acknowledgement arrives', async () => {
    const older = deferred<KeyboardState>();
    keyboard.updateSettings = async settings => { updates.push(settings); return updates.length === 1 ? older.promise : { ...current, sequence: 2, settings }; };
    show();
    const language = await screen.findByRole('textbox', { name: 'Recognition language' });
    fireEvent.change(language, { target: { value: 'en' } });
    expect(updates).toHaveLength(0); expect((language as HTMLInputElement).value).toBe('en');
    expect(screen.getByRole('button', { name: 'Test cloud transcription' }).hasAttribute('disabled')).toBe(true);
    fireEvent.blur(language); await waitFor(() => expect(updates).toHaveLength(1));
    fireEvent.change(language, { target: { value: 'zh' } });
    await act(async () => { older.resolve({ ...current, sequence: 1, settings: updates[0] }); });
    expect((language as HTMLInputElement).value).toBe('zh');
    fireEvent.blur(language); await waitFor(() => expect(updates).toHaveLength(2)); expect(updates[1].language).toBe('zh');
    await screen.findByText('Settings saved automatically');
  });

  test('recognition controls pause during native capture while K4 disable remains immediately available', async () => {
    current.phase = 'recording'; current.audioState.state = 'capturing';
    show();
    const language = await screen.findByRole('textbox', { name: 'Recognition language' });
    expect(language.hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('combobox', { name: 'ASR model' }).getAttribute('aria-disabled')).toBe('true');
    expect(within(screen.getByRole('region', { name: 'Recognition and beautification' })).getByText('Recognition settings can be changed after the current recording or processing finishes.')).toBeTruthy();
    fireEvent.click(screen.getByRole('switch', { name: 'K4 · Hold to speak' }));
    await waitFor(() => expect(updates).toHaveLength(1)); expect(updates[0].k4Enabled).toBe(false);
  });

  test('IME input does not save partial composition, then saves its final value after blur', async () => {
    show(); const language = await screen.findByRole('textbox', { name: 'Recognition language' });
    fireEvent.compositionStart(language); fireEvent.change(language, { target: { value: 'z' } });
    fireEvent.blur(language); expect(updates).toHaveLength(0);
    fireEvent.change(language, { target: { value: 'zh' } }); fireEvent.compositionEnd(language); fireEvent.blur(language);
    await waitFor(() => expect(updates).toHaveLength(1)); expect(updates[0].language).toBe('zh');
  });

  test('leaving the page flushes its queued completed text without waiting for the debounce', async () => {
    const view = show(); const language = await screen.findByRole('textbox', { name: 'Recognition language' });
    fireEvent.change(language, { target: { value: 'en' } }); expect(updates).toHaveLength(0);
    view.unmount(); await waitFor(() => expect(updates).toHaveLength(1)); expect(updates[0].language).toBe('en');
  });
  test('a focused dictionary keeps its raw trailing line through normalized save acknowledgements', async () => {
    keyboard.updateSettings = async settings => {
      updates.push(settings);
      const normalized = { ...settings, dictionary: settings.dictionary.map(term => term.trim()).filter(Boolean) };
      current = { ...current, sequence: current.sequence + 1, settings: normalized }; return current;
    };
    show(); const dictionary = await screen.findByRole('textbox', { name: 'Recognition dictionary' });
    fireEvent.focus(dictionary); fireEvent.change(dictionary, { target: { value: 'Name\n' } });
    (dictionary as HTMLTextAreaElement).setSelectionRange(5, 5);
    await waitFor(() => expect(updates).toHaveLength(1));
    await screen.findByText('Settings saved automatically');
    expect((dictionary as HTMLTextAreaElement).value).toBe('Name\n');
    expect((dictionary as HTMLTextAreaElement).selectionStart).toBe(5);
    fireEvent.change(dictionary, { target: { value: 'Name\nSecond name' } }); fireEvent.blur(dictionary);
    await waitFor(() => expect(updates).toHaveLength(2));
    expect(updates[1].dictionary).toEqual(['Name', 'Second name']);
    await waitFor(() => expect((dictionary as HTMLTextAreaElement).value).toBe('Name\nSecond name'));
  });

});
