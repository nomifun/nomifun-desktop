import type {
  SystemPermissionEntry,
  SystemPermissionKind,
  SystemPermissionState,
  SystemPermissionStatus,
} from '@/common/adapter/ipcBridge';
import { computerPermissionKindsForActions } from '@/common/chat/computerSurface';
export { computerPermissionKindsForOperation, computerPermissionKindsForActions } from '@/common/chat/computerSurface';

export type PermissionCapabilityTab =
  | 'overview'
  | 'voice-input'
  | 'computer-use'
  | 'browser-use';

export type PermissionCapabilityId =
  | 'voice_input'
  | 'computer_use'
  | 'browser_use'
  | 'notifications'
  | 'local_connections'
  | 'local_files'
  | 'application_automation';

/**
 * Product-wide permission audit. An empty `requiredBeforeUse` is deliberate:
 * it means the capability must not be blocked behind an unrelated blanket OS
 * grant. Browser media is a visible, per-site decision; protected files are a
 * operation-specific OS decision. Full Disk Access and Automation have no
 * process-wide readiness probe and must never be presented as already granted.
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
    requiredBeforeUse: [],
    requestedOnDemand: ['accessibility', 'screen_recording'],
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
  local_files: {
    requiredBeforeUse: [],
    requestedOnDemand: [],
    optionalRecovery: ['full_disk_access'],
  },
  application_automation: {
    requiredBeforeUse: [],
    requestedOnDemand: ['automation'],
    optionalRecovery: [],
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

export const capabilityPermissionsHref = (tab: Exclude<PermissionCapabilityTab, 'overview'> | 'notifications'): string =>
  tab === 'notifications'
    ? '#/settings/system?section=preferences&focus=notifications'
    : `#/settings/permissions?tab=${tab}`;
