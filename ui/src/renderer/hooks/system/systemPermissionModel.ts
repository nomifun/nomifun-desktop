import type {
  SystemPermissionEntry,
  SystemPermissionKind,
  SystemPermissionState,
  SystemPermissionStatus,
} from '@/common/adapter/ipcBridge';

export type PermissionCapabilityTab =
  | 'overview'
  | 'voice-input'
  | 'computer-use'
  | 'browser-use'
  | 'notifications';

export type PermissionCapabilityId =
  | 'voice_input'
  | 'computer_use'
  | 'browser_use'
  | 'notifications'
  | 'local_connections'
  | 'workspace_files';

/**
 * Product-wide permission audit. An empty `requiredBeforeUse` is deliberate:
 * it means the capability must not be blocked behind an unrelated blanket OS
 * grant. Browser media is a visible, per-site decision; protected files are a
 * user-selected path decision and never justify asking everyone for Full Disk
 * Access up front.
 */
export const SYSTEM_PERMISSION_AUDIT: Readonly<Record<PermissionCapabilityId, {
  requiredBeforeUse: readonly SystemPermissionKind[];
  requestedOnDemand: readonly SystemPermissionKind[];
  optionalRecovery: readonly SystemPermissionKind[];
}>> = {
  voice_input: {
    requiredBeforeUse: ['microphone'],
    requestedOnDemand: [],
    optionalRecovery: [],
  },
  computer_use: {
    requiredBeforeUse: ['accessibility', 'screen_recording'],
    requestedOnDemand: [],
    optionalRecovery: [],
  },
  browser_use: {
    requiredBeforeUse: [],
    requestedOnDemand: ['camera', 'microphone', 'location', 'notifications'],
    optionalRecovery: [],
  },
  notifications: {
    requiredBeforeUse: ['notifications'],
    requestedOnDemand: [],
    optionalRecovery: [],
  },
  local_connections: {
    requiredBeforeUse: [],
    requestedOnDemand: ['local_network'],
    optionalRecovery: [],
  },
  workspace_files: {
    requiredBeforeUse: [],
    requestedOnDemand: [],
    optionalRecovery: ['full_disk_access'],
  },
};

export const systemPermissionEntry = (
  status: SystemPermissionStatus | null,
  kind: SystemPermissionKind
): SystemPermissionEntry | undefined =>
  status?.permissions.find((permission) => permission.kind === kind);

const permissionStateIsReady = (state: SystemPermissionState | undefined): boolean =>
  state === 'granted' || state === 'not_required';

export const permissionEntryIsReady = (entry: SystemPermissionEntry | undefined): boolean =>
  permissionStateIsReady(entry?.state);

/** Exact action-time Computer prerequisites; `computer/launch` needs neither. */
export const computerPermissionKindsForActions = (
  actionIds: Iterable<string>
): Array<'accessibility' | 'screen_recording'> => {
  const actions = new Set([...actionIds].filter((actionId) => actionId.startsWith('computer/')));
  const required: Array<'accessibility' | 'screen_recording'> = [];
  if (actions.size === 0 || actions.has('computer/observe')) required.push('screen_recording');
  if (
    actions.size === 0 ||
    actions.has('computer/a11y.observe') ||
    actions.has('computer/input')
  ) {
    required.push('accessibility');
  }
  return required;
};

export const computerPermissionsReady = (
  status: SystemPermissionStatus | null,
  actionIds: Iterable<string>
): boolean =>
  missingComputerPermissionKinds(status, actionIds).length === 0;

export const missingComputerPermissionKinds = (
  status: SystemPermissionStatus | null,
  actionIds: Iterable<string>
): Array<'accessibility' | 'screen_recording'> =>
  computerPermissionKindsForActions(actionIds).filter(
    (kind) => !permissionEntryIsReady(systemPermissionEntry(status, kind))
  );

export const capabilityPermissionsHref = (tab: Exclude<PermissionCapabilityTab, 'overview'>): string =>
  `#/settings/permissions?tab=${tab}`;
