import { describe, expect, test } from 'bun:test';
import type { SystemPermissionStatus } from '@/common/adapter/ipcBridge';
import {
  SYSTEM_PERMISSION_AUDIT,
  capabilityPermissionsHref,
  computerPermissionKindsForActions,
  computerPermissionsReady,
  missingComputerPermissionKinds,
} from './systemPermissionModel';

const status = (accessibility: 'granted' | 'not_determined', screen: 'granted' | 'not_determined'): SystemPermissionStatus => ({
  platform: 'macos',
  app_label: 'NomiFun',
  permissions: [
    {
      kind: 'microphone', state: 'granted', can_request: false, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['voice_input'],
    },
    {
      kind: 'accessibility', state: accessibility, can_request: accessibility !== 'granted', can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['computer_use'],
    },
    {
      kind: 'screen_recording', state: screen, can_request: screen !== 'granted', can_open_settings: true,
      requires_restart_after_grant: true, capabilities: ['computer_use'],
    },
  ],
});

describe('system permission product model', () => {
  test('keeps up-front grants distinct from per-site and optional recovery access', () => {
    expect(SYSTEM_PERMISSION_AUDIT.voice_input.requiredBeforeUse).toEqual(['microphone']);
    expect(SYSTEM_PERMISSION_AUDIT.computer_use.requiredBeforeUse).toEqual([
      'accessibility',
      'screen_recording',
    ]);
    expect(SYSTEM_PERMISSION_AUDIT.browser_use.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.browser_use.requestedOnDemand).toEqual([
      'camera',
      'microphone',
      'location',
      'notifications',
    ]);
    expect(SYSTEM_PERMISSION_AUDIT.local_connections.requestedOnDemand).toEqual(['local_network']);
    expect(SYSTEM_PERMISSION_AUDIT.workspace_files.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.workspace_files.optionalRecovery).toEqual(['full_disk_access']);
  });

  test('derives only the Computer permissions required by frozen actions', () => {
    expect(computerPermissionKindsForActions(['computer/launch'])).toEqual([]);
    expect(computerPermissionKindsForActions(['computer/observe'])).toEqual(['screen_recording']);
    expect(computerPermissionKindsForActions(['computer/a11y.observe'])).toEqual(['accessibility']);
    expect(computerPermissionKindsForActions(['computer/input'])).toEqual(['accessibility']);
    expect(computerPermissionKindsForActions([])).toEqual(['screen_recording', 'accessibility']);
    expect(computerPermissionKindsForActions(['browser/observe'])).toEqual(['screen_recording', 'accessibility']);
  });

  test('reports action-time Computer readiness without deciding Session admission', () => {
    expect(computerPermissionsReady(status('granted', 'granted'), [])).toBe(true);
    expect(computerPermissionsReady(status('not_determined', 'granted'), [])).toBe(false);
    expect(computerPermissionsReady(status('granted', 'not_determined'), ['computer/observe'])).toBe(false);
    expect(computerPermissionsReady(status('not_determined', 'granted'), ['computer/launch'])).toBe(true);
    expect(missingComputerPermissionKinds(status('not_determined', 'granted'), [])).toEqual(['accessibility']);
    expect(missingComputerPermissionKinds(status('granted', 'not_determined'), ['computer/observe'])).toEqual(['screen_recording']);
  });

  test('builds direct HashRouter recovery links for contextual prompts', () => {
    expect(capabilityPermissionsHref('voice-input')).toBe('#/settings/permissions?tab=voice-input');
    expect(capabilityPermissionsHref('computer-use')).toBe('#/settings/permissions?tab=computer-use');
    expect(capabilityPermissionsHref('browser-use')).toBe('#/settings/permissions?tab=browser-use');
  });
});
