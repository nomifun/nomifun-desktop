import { describe, expect, test } from 'bun:test';
import {
  SYSTEM_PERMISSION_AUDIT,
  capabilityPermissionsHref,
} from './systemPermissionModel';

describe('system permission product model', () => {
  test('keeps up-front grants distinct from per-site and optional recovery access', () => {
    expect(SYSTEM_PERMISSION_AUDIT.voice_input.requiredBeforeUse).toEqual(['microphone']);
    expect(SYSTEM_PERMISSION_AUDIT.computer_use.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.browser_use.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.browser_use.requestedOnDemand).toEqual([
      'camera',
      'microphone',
      'location',
      'notifications',
    ]);
    expect(SYSTEM_PERMISSION_AUDIT.local_connections.requestedOnDemand).toEqual(['local_network']);
    expect(SYSTEM_PERMISSION_AUDIT.local_files.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.local_files.optionalRecovery).toEqual(['full_disk_access']);
    expect(SYSTEM_PERMISSION_AUDIT.application_automation.requiredBeforeUse).toEqual([]);
    expect(SYSTEM_PERMISSION_AUDIT.application_automation.requestedOnDemand).toEqual(['automation']);
  });

  test('builds direct HashRouter recovery links for contextual prompts', () => {
    expect(capabilityPermissionsHref('voice-input')).toBe('#/settings/permissions?tab=voice-input');
    expect(capabilityPermissionsHref('computer-use')).toBe('#/settings/permissions?tab=computer-use');
    expect(capabilityPermissionsHref('browser-use')).toBe('#/settings/permissions?tab=browser-use');
    expect(capabilityPermissionsHref('notifications')).toBe('#/settings/system?section=preferences&focus=notifications');
  });
});
