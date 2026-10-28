import { describe, expect, test } from 'bun:test';
import type { SystemPermissionState } from '@/common/adapter/ipcBridge';
import type { KeyboardSettings, KeyboardState } from '@/common/keyboard';
import { deriveKeyboardReadiness, type KeyboardReadinessInput } from './keyboardReadiness';

const settings = (): KeyboardSettings => ({
  enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true,
  audioDeviceId: null, asrMode: 'streaming',
  asrModel: { providerId: 'provider-1', model: 'streaming-model' }, textModel: null,
  language: '', dictionary: [], beautifyInstruction: '', alwaysOnTop: false, subtitleFontSize: 14,
});

function input(): KeyboardReadinessInput {
  const saved = settings();
  const snapshot = {
    sequence: 1, cancelGeneration: 0, settings: saved,
    documentId: null, revision: 0, operationId: null, editSessionId: null,
    dictationSelection: null, phase: 'idle' as const, text: '', originalText: '',
    partial: '', candidate: '', visible: false, pendingAction: null, error: null, notice: null,
    audioState: { state: 'stopped' as const, detail: null }, capturePreparing: false,
    microphoneTest: {
      id: null, deviceId: null, state: 'idle' as const, callbackObserved: false,
      samples: 0, nonzero: 0, peak: 0, rms: 0, elapsedMs: 0, stopped: true, detail: null,
    },
    device: {
      connected: true, id: 'keyboard-1', port: 'test-port', firmware: 'test-firmware',
      policyState: 'applied' as const, captureState: 'stopped' as const, detail: null,
    },
    service: { running: true, ready: true, detail: null },
  };
  const state: KeyboardState = snapshot;
  return {
    state, draft: structuredClone(saved), saving: false, saveFailed: false, permission: 'granted',
    catalog: {
      asr: [{ providerId: 'provider-1', model: 'streaming-model', label: 'Streaming', protocol: 'streaming' }],
      platformAsr: [{ providerId: 'provider-2', model: 'file-model', label: 'File ASR', protocol: 'upload' }],
      text: [],
    },
    audioDevices: [{ id: 'mic-1', label: 'Microphone', isDefault: false }],
  };
}

describe('derived keyboard readiness', () => {
  test('reports independent hardware and speech prerequisites without requiring text polish', () => {
    expect(deriveKeyboardReadiness(input())).toEqual({
      hardwareReady: true, permissionBlocked: false, deviceAvailable: true,
      savedModelAvailable: true, cloudBlockers: [], modelCatalogueEmpty: false,
      platformAlternative: false,
    });
  });

  test.each(['enabled', 'k4Enabled', 'connected', 'policy', 'service'] as const)(
    'hardware readiness requires %s without blocking an independent cloud test', (gate) => {
      const value = input();
      if (gate === 'enabled' || gate === 'k4Enabled') value.state.settings[gate] = false;
      else if (gate === 'connected') value.state.device.connected = false;
      else if (gate === 'policy') value.state.device.policyState = 'pending';
      else value.state.service.ready = false;
      const result = deriveKeyboardReadiness(value);
      expect(result.hardwareReady).toBe(false);
      expect(result.cloudBlockers).toEqual([]);
    }
  );

  test('an errored device policy is not applied hardware readiness', () => {
    const value = input();
    value.state.device.policyState = 'error';
    expect(deriveKeyboardReadiness(value).hardwareReady).toBe(false);
  });

  test.each([
    ['denied', true], ['restricted', true], ['granted', false], ['not_determined', false],
    ['not_granted', false], ['not_required', false], ['unknown', false], [undefined, false],
  ] satisfies Array<[SystemPermissionState | undefined, boolean]>)(
    'permission %s blocks only an explicit refusal or restriction', (permission, blocked) => {
      const value = input();
      value.permission = permission;
      const result = deriveKeyboardReadiness(value);
      expect(result.permissionBlocked).toBe(blocked);
      expect(result.cloudBlockers).toEqual(blocked ? ['permission'] : []);
    }
  );

  test.each([
    { selected: null, devices: undefined, available: false },
    { selected: null, devices: [], available: false },
    { selected: null, devices: [{ id: 'mic-1', label: 'Microphone', isDefault: false }], available: true },
    { selected: 'mic-1', devices: [{ id: 'mic-1', label: 'Microphone', isDefault: false }], available: true },
    { selected: 'missing', devices: [{ id: 'mic-1', label: 'missing', isDefault: true }], available: false },
    { selected: 'MIC-1', devices: [{ id: 'mic-1', label: 'Microphone', isDefault: true }], available: false },
  ])('validates the draft input reference against the native inventory: $selected / $available', ({ selected, devices, available }) => {
    const value = input();
    value.draft.audioDeviceId = selected;
    value.audioDevices = devices;
    const result = deriveKeyboardReadiness(value);
    expect(result.deviceAvailable).toBe(available);
    expect(result.cloudBlockers).toEqual(available ? [] : ['device']);
  });

  test('cloud testing also requires the saved input even while the draft input is available', () => {
    const value = input();
    value.state.settings.audioDeviceId = 'missing-saved-device';
    value.draft.audioDeviceId = 'mic-1';
    value.saving = true;
    const result = deriveKeyboardReadiness(value);
    expect(result.deviceAvailable).toBe(true);
    expect(result.cloudBlockers).toEqual(['device', 'saving']);
  });

  test.each(['no-selection', 'no-catalogue', 'wrong-provider', 'wrong-model', 'wrong-mode'] as const)(
    'cloud testing rejects a saved model with %s', (cause) => {
      const value = input();
      if (cause === 'no-selection') value.state.settings.asrModel = null;
      else if (cause === 'no-catalogue') value.catalog = undefined;
      else if (cause === 'wrong-provider') value.state.settings.asrModel!.providerId = 'provider-other';
      else if (cause === 'wrong-model') value.state.settings.asrModel!.model = 'STREAMING-MODEL';
      else value.state.settings.asrMode = 'platform';
      const result = deriveKeyboardReadiness(value);
      expect(result.savedModelAvailable).toBe(false);
      expect(result.cloudBlockers).toEqual(['model']);
    }
  );

  test('a platform saved model passes only against the platform catalogue', () => {
    const value = input();
    value.state.settings.asrMode = 'platform';
    value.state.settings.asrModel = { providerId: 'provider-2', model: 'file-model' };
    expect(deriveKeyboardReadiness(value).savedModelAvailable).toBe(true);
    expect(deriveKeyboardReadiness(value).cloudBlockers).toEqual([]);
  });

  test('draft model changes keep the saved reference separate and wait for saving acknowledgement', () => {
    const value = input();
    value.draft.asrMode = 'platform';
    value.draft.asrModel = null;
    value.saving = true;
    const result = deriveKeyboardReadiness(value);
    expect(result.savedModelAvailable).toBe(true);
    expect(result.cloudBlockers).toEqual(['saving']);
  });

  test('reports every prerequisite reason in a stable order', () => {
    const value = input();
    value.permission = 'denied';
    value.audioDevices = [];
    value.state.settings.asrModel = null;
    value.saving = true;
    expect(deriveKeyboardReadiness(value).cloudBlockers).toEqual(['permission', 'device', 'model', 'saving']);
  });

  test('offers the ordinary ASR alternative for an empty streaming catalogue', () => {
    const value = input();
    value.catalog!.asr = [];
    const result = deriveKeyboardReadiness(value);
    expect(result.modelCatalogueEmpty).toBe(true);
    expect(result.platformAlternative).toBe(true);
  });

  test('uses the draft mode for empty catalogue guidance without changing saved readiness', () => {
    const value = input();
    value.catalog!.platformAsr = [];
    value.draft.asrMode = 'platform';
    const result = deriveKeyboardReadiness(value);
    expect(result.modelCatalogueEmpty).toBe(true);
    expect(result.platformAlternative).toBe(false);
    expect(result.savedModelAvailable).toBe(true);
  });

  test('missing catalogue data provides no available model or platform alternative', () => {
    const value = input();
    value.catalog = undefined;
    const result = deriveKeyboardReadiness(value);
    expect(result.modelCatalogueEmpty).toBe(true);
    expect(result.platformAlternative).toBe(false);
    expect(result.savedModelAvailable).toBe(false);
  });

  test('does not mutate a user selection or native snapshot while deriving readiness', () => {
    const value = input();
    value.draft.asrModel = null;
    value.draft.audioDeviceId = 'missing';
    value.saving = true;
    const before = structuredClone(value);
    deriveKeyboardReadiness(value);
    expect(value).toEqual(before);
  });
  test('a failed automatic save is a stable explicit cloud blocker', () => {
    const value = input(); value.saving = true; value.saveFailed = true;
    expect(deriveKeyboardReadiness(value).cloudBlockers).toEqual(['saveFailed']);
  });

});
