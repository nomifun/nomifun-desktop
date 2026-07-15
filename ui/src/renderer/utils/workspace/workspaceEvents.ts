import type { SessionTarget } from '@/common/types/ids';

export const WORKSPACE_TOGGLE_EVENT = 'nomifun-workspace-toggle';
export const WORKSPACE_STATE_EVENT = 'nomifun-workspace-state';
export const WORKSPACE_HAS_FILES_EVENT = 'nomifun-workspace-has-files';

export interface WorkspaceStateDetail {
  target: SessionTarget;
  collapsed: boolean;
}

export interface WorkspaceHasFilesDetail {
  target: SessionTarget;
  hasFiles: boolean;
  /**
   * True when this signal corresponds to the workspace tree's first load for
   * this session. Lets listeners distinguish backend-seeded files
   * (rules/skills present from the start) from files that appear mid-session.
   *
   * Note: a fresh tree mount counts as initial — switching away from a
   * conversation and back will report `isInitial: true` again, so files added
   * while the conversation was unmounted are not detectable here.
   */
  isInitial: boolean;
}

export interface WorkspaceToggleDetail {
  target: SessionTarget;
}

export function dispatchWorkspaceToggleEvent(target: SessionTarget) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspaceToggleDetail>(WORKSPACE_TOGGLE_EVENT, { detail: { target } })
  );
}

export function dispatchWorkspaceStateEvent(target: SessionTarget, collapsed: boolean) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspaceStateDetail>(WORKSPACE_STATE_EVENT, { detail: { target, collapsed } })
  );
}

/**
 * 当工作空间文件状态变化时触发
 * Dispatch when workspace files status changes
 */
export function dispatchWorkspaceHasFilesEvent(target: SessionTarget, hasFiles: boolean, isInitial: boolean) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspaceHasFilesDetail>(WORKSPACE_HAS_FILES_EVENT, {
      detail: { target, hasFiles, isInitial },
    })
  );
}
