import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isTauriRuntime } from './adapter/tauriRuntime';

export interface KeyboardModel {
  providerId: string;
  model: string;
}

export interface KeyboardCatalogModel extends KeyboardModel { label: string; protocol: string }
export interface KeyboardModelCatalog { asr: KeyboardCatalogModel[]; platformAsr: KeyboardCatalogModel[]; text: KeyboardCatalogModel[] }
export interface KeyboardAudioDevice { id: string; label: string; isDefault: boolean }
export interface KeyboardSelection { anchor: number; focus: number }
export type KeyboardAsrMode = 'streaming' | 'platform';

export interface KeyboardSettings {
  enabled: boolean;
  k2Enabled: boolean;
  k3Enabled: boolean;
  k4Enabled: boolean;
  audioDeviceId: string | null;
  asrMode: KeyboardAsrMode;
  asrModel: KeyboardModel | null;
  textModel: KeyboardModel | null;
  language: string;
  dictionary: string[];
  beautifyInstruction: string;
  alwaysOnTop: boolean;
  subtitleFontSize: number;
  launchAtLogin: boolean;
}

/** Native snapshots are authoritative; the renderer never predicts policy or capture acknowledgements. */
export interface KeyboardState {
  sequence: number;
  cancelGeneration: number;
  settings: KeyboardSettings;
  documentId: string | null;
  revision: number;
  operationId: string | null;
  editSessionId: string | null;
  dictationSelection: KeyboardSelection | null;
  phase: 'idle' | 'recording' | 'finalizing' | 'beautifying';
  text: string;
  originalText: string;
  partial: string;
  candidate: string;
  visible: boolean;
  pendingAction: 'beautify' | null;
  error: string | null;
  notice: string | null;
  audioState: { state: 'stopped' | 'starting' | 'capturing' | 'stopping' | 'error'; detail: string | null };
  capturePreparing: boolean;
  device: {
    connected: boolean;
    id: string | null;
    port: string | null;
    firmware: string | null;
    policyState: 'pending' | 'applied' | 'error';
    captureState: 'stopped' | 'capturing' | 'stopping' | 'unknown';
    detail: string | null;
  };
  service: { running: boolean; ready: boolean; detail: string | null };
}

export type KeyboardAction = 'stop' | 'cancel' | 'beautify' | 'restore_original' | 'close'
  | 'open' | 'reset_position' | 'test_start' | 'test_stop' | 'quit_all' | 'toggle_pin'
  | 'edit_begin' | 'edit' | 'edit_end' | 'edit_cancel';
export interface KeyboardActionRequest {
  action: KeyboardAction;
  documentId?: string | null;
  revision?: number;
  cancelGeneration?: number;
  editId?: string;
  text?: string;
  selection?: KeyboardSelection | null;
}
export interface KeyboardEditFlush { documentId: string; editId: string }

export const keyboard = {
  available: isTauriRuntime,
  getState: () => invoke<KeyboardState>('keyboard_get_state'),
  getCatalog: () => invoke<KeyboardModelCatalog>('keyboard_model_catalog'),
  getAudioDevices: () => invoke<KeyboardAudioDevice[]>('keyboard_audio_devices'),
  updateSettings: (settings: KeyboardSettings) => invoke<KeyboardState>('keyboard_update_settings', { settings }),
  action: (request: KeyboardActionRequest) => invoke<KeyboardState>('keyboard_action', { ...request }),
  listen: (callback: (state: KeyboardState) => void) => listen<KeyboardState>('keyboard://state', (event) => callback(event.payload)),
  listenEditFlush: (callback: (request: KeyboardEditFlush) => void) => listen<KeyboardEditFlush>('keyboard://edit-flush', (event) => callback(event.payload)),
};

export const modelSelectionKey = (model: KeyboardModel | null): string => model ? JSON.stringify([model.providerId, model.model]) : '';
export function parseModelSelection(value: string): KeyboardModel | null {
  if (!value) return null;
  const pair: unknown = JSON.parse(value);
  if (!Array.isArray(pair) || pair.length !== 2 || !pair.every((item) => typeof item === 'string')) throw new Error('Invalid model selection');
  return { providerId: pair[0], model: pair[1] };
}

export function documentAction(state: KeyboardState, action: KeyboardAction): KeyboardActionRequest {
  return { action, documentId: state.documentId, revision: state.revision, cancelGeneration: state.cancelGeneration };
}
