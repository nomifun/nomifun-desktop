import frozen from '../../../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json';

export type ComputerPermissionKind = 'accessibility' | 'screen_recording';
export type ComputerActionGroup = 'observation' | 'interaction' | 'application' | 'clipboard';
type FrozenComputerAction = {
  action_id: string;
  driver_tool: string;
  group: ComputerActionGroup;
  system_permissions: {
    accessibility: boolean;
    screen_recording: 'never' | 'always' | 'window_screenshot' | 'batch_screenshot';
  };
};

/** The frozen driver surface is the only source of Computer identities and prerequisites. */
export const computerTools = frozen.tools as unknown as readonly FrozenComputerAction[];
const byAction = new Map(computerTools.map(tool => [tool.action_id, tool]));
export const computerActionGroup = (actionId: string): ComputerActionGroup | undefined => byAction.get(actionId)?.group;
export const computerActionGroups: readonly ComputerActionGroup[] = ['observation', 'interaction', 'application', 'clipboard'];

export const computerPermissionKindsForOperation = (
  actionId: unknown,
  operation: unknown,
  unconditionalOnly = false,
): ComputerPermissionKind[] => {
  const tool = typeof actionId === 'string' ? byAction.get(actionId) : undefined;
  if (!tool || tool.driver_tool !== operation || !tool.system_permissions) return [];
  const permissions: ComputerPermissionKind[] = [];
  if (tool.system_permissions.accessibility) permissions.push('accessibility');
  const screen = tool.system_permissions.screen_recording;
  if (screen === 'always' || !unconditionalOnly && screen !== 'never') permissions.push('screen_recording');
  return permissions;
};

export const computerPermissionKindsForActions = (actionIds: Iterable<string>): ComputerPermissionKind[] => {
  const permissions = new Set<ComputerPermissionKind>();
  for (const actionId of actionIds) {
    const tool = byAction.get(actionId);
    if (tool) computerPermissionKindsForOperation(actionId, tool.driver_tool).forEach(kind => permissions.add(kind));
  }
  return [...permissions].sort();
};
