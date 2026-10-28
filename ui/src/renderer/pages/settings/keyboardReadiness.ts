import type { SystemPermissionState } from '@/common/adapter/ipcBridge';
import type { KeyboardAudioDevice, KeyboardModelCatalog, KeyboardSettings, KeyboardState } from '@/common/keyboard';

export interface KeyboardReadinessInput {
  state: KeyboardState;
  draft: KeyboardSettings;
  catalog: KeyboardModelCatalog | undefined;
  audioDevices: readonly KeyboardAudioDevice[] | undefined;
  permission: SystemPermissionState | undefined;
  saving: boolean;
  saveFailed: boolean;
}

export type KeyboardCloudBlocker = 'permission' | 'device' | 'model' | 'saving' | 'saveFailed';

export interface KeyboardReadiness {
  hardwareReady: boolean;
  permissionBlocked: boolean;
  deviceAvailable: boolean;
  savedModelAvailable: boolean;
  cloudBlockers: KeyboardCloudBlocker[];
  modelCatalogueEmpty: boolean;
  platformAlternative: boolean;
}

export function deriveKeyboardReadiness({
  state, draft, catalog, audioDevices, permission, saving, saveFailed,
}: KeyboardReadinessInput): KeyboardReadiness {
  const deviceIsAvailable = (deviceId: string | null): boolean => deviceId === null
    ? Boolean(audioDevices?.length)
    : Boolean(audioDevices?.some((device) => device.id === deviceId));
  const deviceAvailable = deviceIsAvailable(draft.audioDeviceId);
  const permissionBlocked = permission === 'denied' || permission === 'restricted';
  const saved = state.settings;
  const savedModel = saved.asrModel;
  const savedModels = saved.asrMode === 'platform' ? catalog?.platformAsr : catalog?.asr;
  const savedModelAvailable = Boolean(savedModel && savedModels?.some((model) =>
    model.providerId === savedModel.providerId && model.model === savedModel.model));
  const draftModels = draft.asrMode === 'platform' ? catalog?.platformAsr : catalog?.asr;
  const modelCatalogueEmpty = !draftModels?.length;
  const cloudBlockers: KeyboardCloudBlocker[] = [];
  if (permissionBlocked) cloudBlockers.push('permission');
  if (!deviceAvailable || !deviceIsAvailable(saved.audioDeviceId)) cloudBlockers.push('device');
  if (!savedModelAvailable) cloudBlockers.push('model');
  if (saveFailed) cloudBlockers.push('saveFailed');
  else if (saving) cloudBlockers.push('saving');

  return {
    hardwareReady: saved.enabled && saved.k4Enabled && state.device.connected
      && state.device.policyState === 'applied' && state.service.ready,
    permissionBlocked,
    deviceAvailable,
    savedModelAvailable,
    cloudBlockers,
    modelCatalogueEmpty,
    platformAlternative: draft.asrMode === 'streaming' && modelCatalogueEmpty && Boolean(catalog?.platformAsr.length),
  };
}
