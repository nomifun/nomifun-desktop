import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { act } from 'react';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { keyboard, type KeyboardActionRequest, type KeyboardSettings as Settings, type KeyboardState } from '@/common/keyboard';
import en from '@/renderer/services/i18n/locales/en-US/settings.json';
import KeyboardSettings from './KeyboardSettings';

const original = { ...keyboard };
let current: KeyboardState;
let actions: KeyboardActionRequest[];
let updates: Settings[];
let listener: ((state: KeyboardState) => void) | undefined;
beforeEach(() => {
  current = {
    settings: { enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true, audioDeviceId: null, asrMode: 'streaming',
      asrModel: { providerId: 'provider-1', model: 'streaming-model' }, textModel: null,
      language: '', dictionary: [], beautifyInstruction: '', alwaysOnTop: false, subtitleFontSize: 14, launchAtLogin: false },
    sequence: 0, cancelGeneration: 0, documentId: 'doc-1', revision: 2, operationId: null, editSessionId: null, dictationSelection: null, phase: 'idle', text: 'Ready text', originalText: 'Ready text',
    partial: '', candidate: '', visible: false, pendingAction: null, error: null, notice: null,
    audioState: { state: 'stopped', detail: null },
    capturePreparing: false,
    device: { connected: false, id: null, port: null, firmware: null,
      policyState: 'applied', captureState: 'stopped', detail: null },
    service: { running: false, ready: false, detail: null },
  };
  actions = []; updates = []; listener = undefined;
  keyboard.available = () => true;
  keyboard.getState = async () => current;
  keyboard.listen = async callback => { listener = callback; return () => { listener = undefined; }; };
  keyboard.getCatalog = async () => ({
    asr: [{ providerId: 'provider-1', model: 'streaming-model', label: 'Streaming model', protocol: 'streaming' }],
    platformAsr: [{ providerId: 'provider-1', model: 'file-model', label: 'File recognition model', protocol: 'upload' }], text: [],
  });
  keyboard.getAudioDevices = async () => [{ id: 'microphone-1', label: 'Desk microphone', isDefault: false }];
  keyboard.action = async (request) => { actions.push(request); return current; };
  keyboard.updateSettings = async (settings) => { updates.push(settings); return { ...current, settings }; };
});
afterEach(() => { cleanup(); Object.assign(keyboard, original); });
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings: { keyboard: en.keyboard } } } } });
const show = () => render(<SWRConfig value={{ provider: () => new Map() }}><I18nextProvider i18n={i18n}><MemoryRouter><KeyboardSettings /></MemoryRouter></I18nextProvider></SWRConfig>);

describe('system microphone keyboard settings', () => {
  test('the explicit microphone test works without a connected keyboard or device service', async () => {
    show();
    const start = await screen.findByRole('button', { name: 'Test microphone' });
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
    fireEvent.click(screen.getByRole('button', { name: 'Finish test' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('a previous hardware recognition error does not become a settings failure', async () => {
    current.error = 'Recognition failed. Check the provider connection and try again.';
    show();
    const start = await screen.findByRole('button', { name: 'Test microphone' });
    expect(screen.queryByText(current.error)).toBeNull();
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
    fireEvent.click(await screen.findByRole('button', { name: 'Test microphone' }));
    await screen.findByText('Recording system microphone');
    const failure = 'The requested test connection failed. Try again.';
    act(() => {
      current = { ...current, sequence: 2, phase: 'idle', operationId: null, error: failure, audioState: { state: 'stopped', detail: null } };
      listener?.(current);
    });
    await screen.findByText(failure);
    expect(screen.getByRole('button', { name: 'Test microphone' }).hasAttribute('disabled')).toBe(false);
    act(() => {
      current = { ...current, sequence: 3, error: 'Unrelated later hardware failure' };
      listener?.(current);
    });
    expect(screen.queryByText('Unrelated later hardware failure')).toBeNull();
    expect(screen.getByText(failure)).toBeTruthy();
  });

  test('a replaced operation cannot report its hardware failure as a microphone test result', async () => {
    keyboard.action = async request => {
      actions.push(request);
      current = { ...current, sequence: 1, capturePreparing: true, operationId: 'test-operation' };
      return current;
    };
    show();
    fireEvent.click(await screen.findByRole('button', { name: 'Test microphone' }));
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
    expect(screen.queryByText('Later hardware failure')).toBeNull();
  });

  test('a settings mutation failure still appears and leaves the saved switch unchanged', async () => {
    keyboard.updateSettings = async () => { throw new Error('The keyboard settings could not be saved.'); };
    show();
    const toggle = await screen.findByRole('switch', { name: 'K4 · Hold to speak' });
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(toggle);
    await screen.findByText('The keyboard settings could not be saved.');
    expect(toggle.getAttribute('aria-checked')).toBe('true');
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
    expect(screen.getByRole('button', { name: 'Test microphone' }).hasAttribute('disabled')).toBe(true);
    const stop = screen.getByRole('button', { name: 'Finish test' });
    expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('waiting for the editor to save can be stopped before the microphone starts', async () => {
    current.capturePreparing = true;
    show();
    await screen.findByText('System recording stopped');
    expect(screen.getByRole('button', { name: 'Test microphone' }).hasAttribute('disabled')).toBe(true);
    const stop = screen.getByRole('button', { name: 'Finish test' });
    expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions).toEqual([{ action: 'test_stop', documentId: 'doc-1', revision: 2 }]));
  });

  test('a selected system microphone is saved as a device reference in the native settings', async () => {
    show();
    await screen.findByText('System recording stopped');
    const select = screen.getByRole('combobox', { name: 'Recording device' });
    fireEvent.click(select);
    fireEvent.click(await screen.findByText('Desk microphone'));
    const save = screen.getByRole('button', { name: 'Save settings' });
    await waitFor(() => expect(save.hasAttribute('disabled')).toBe(false));
    fireEvent.click(save);
    await waitFor(() => expect(updates.length).toBe(1));
    expect(updates[0].audioDeviceId).toBe('microphone-1');
    expect(updates[0].asrModel).toEqual(current.settings.asrModel);
  });

  test('switching to standard ASR filters models, clears an incompatible reference and saves the explicit mode', async () => {
    show();
    const mode = await screen.findByRole('combobox', { name: 'Recognition mode' });
    fireEvent.click(mode);
    fireEvent.click(await screen.findByText('Recognize after recording (standard ASR)'));
    await screen.findByText('Only configured cloud models that accept completed audio uploads are listed. This mode does not show live transcription.');
    expect(screen.getByRole('button', { name: 'Test microphone' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('combobox', { name: 'ASR model' }));
    const fileModel = await screen.findByText('File recognition model');
    expect(screen.queryByText('Streaming model')).toBeNull();
    fireEvent.click(fileModel);
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(updates.length).toBe(1));
    expect(updates[0].asrMode).toBe('platform');
    expect(updates[0].asrModel).toEqual({ providerId: 'provider-1', model: 'file-model' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Test microphone' }).hasAttribute('disabled')).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Test microphone' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'test_start' }]));
  });

  test('switching back to streaming requires a streaming-capable model', async () => {
    current.settings.asrMode = 'platform';
    current.settings.asrModel = { providerId: 'provider-1', model: 'file-model' };
    show();
    fireEvent.click(await screen.findByRole('combobox', { name: 'Recognition mode' }));
    fireEvent.click(await screen.findByText('Live streaming recognition'));
    fireEvent.click(screen.getByRole('combobox', { name: 'ASR model' }));
    const streamingModel = await screen.findByText('Streaming model');
    expect(screen.queryByText('File recognition model')).toBeNull();
    fireEvent.click(streamingModel);
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(updates.length).toBe(1));
    expect(updates[0].asrMode).toBe('streaming');
    expect(updates[0].asrModel).toEqual({ providerId: 'provider-1', model: 'streaming-model' });
  });

  test('subtitle font loads from native settings and discard returns to that saved size', async () => {
    current.settings.subtitleFontSize = 20;
    show();
    const font = await screen.findByRole('combobox', { name: 'Subtitle font size' });
    expect(font.textContent).toContain('20px');
    fireEvent.click(font); fireEvent.click(await screen.findByText('12px'));
    expect(font.textContent).toContain('12px');
    expect(updates).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Discard unsaved changes' }));
    await waitFor(() => expect(font.textContent).toContain('20px'));
    expect(updates).toHaveLength(0);
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
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(updates).toHaveLength(1));
    expect(updates[0].subtitleFontSize).toBe(32);
    expect(updates[0].asrModel).toEqual(current.settings.asrModel);
    expect(updates[0].audioDeviceId).toBe(current.settings.audioDeviceId);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save settings' }).hasAttribute('disabled')).toBe(true));
  });
});
