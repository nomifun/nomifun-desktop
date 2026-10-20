import { describe, expect, test } from 'bun:test';
import { computerActionGroup, computerActionGroups, computerPermissionKindsForActions, computerPermissionKindsForOperation, computerTools } from './computerSurface';
import { normalizeSystemPermissionRequired } from './chatLib';

describe('frozen Computer permission display', () => {
  test('groups the entire frozen action surface without inventing actions', () => {
    expect(new Set(computerTools.map(tool => tool.action_id)).size).toBe(22);
    expect(computerTools.every(tool => computerActionGroups.includes(computerActionGroup(tool.action_id)!))).toBe(true);
    expect(computerPermissionKindsForActions([])).toEqual([]);
    expect(computerPermissionKindsForActions(['unregistered/action'])).toEqual([]);
  });

  test('accepts a missing permission subset only for its admitted action and driver operation', () => {
    const tool = computerTools.find(tool => tool.system_permissions.accessibility && tool.system_permissions.screen_recording === 'window_screenshot')!;
    const request = { schema: 'system-permission-required.v1', capability: 'computer', action_id: tool.action_id,
      operation: tool.driver_tool, permissions: ['screen_recording'], execution: 'not_executed' };
    expect(normalizeSystemPermissionRequired(request, tool.action_id)?.permissions).toEqual(['screen_recording']);
    expect(normalizeSystemPermissionRequired(request, 'foreign/action')).toBeUndefined();
    expect(normalizeSystemPermissionRequired({ ...request, operation: 'foreign_operation' }, tool.action_id)).toBeUndefined();
    expect(normalizeSystemPermissionRequired({ ...request, permissions: ['screen_recording', 'screen_recording'] }, tool.action_id)).toBeUndefined();
  });

  test('a successful conditional screenshot action cannot prove Screen Recording readiness', () => {
    const tool = computerTools.find(tool => tool.system_permissions.screen_recording === 'window_screenshot')!;
    expect(computerPermissionKindsForOperation(tool.action_id, tool.driver_tool)).toContain('screen_recording');
    expect(computerPermissionKindsForOperation(tool.action_id, tool.driver_tool, true)).not.toContain('screen_recording');
  });
});
