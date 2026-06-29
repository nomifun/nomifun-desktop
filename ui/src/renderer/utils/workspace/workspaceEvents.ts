export const WORKSPACE_TOGGLE_EVENT = 'nomifun-workspace-toggle';
export const WORKSPACE_STATE_EVENT = 'nomifun-workspace-state';
export const WORKSPACE_HAS_FILES_EVENT = 'nomifun-workspace-has-files';
export const WORKSPACE_AVAILABILITY_EVENT = 'nomifun-workspace-availability';

export interface WorkspaceStateDetail {
  collapsed: boolean;
}

/**
 * Availability signal for the titlebar workspace toggle. Some routes (notably
 * the orchestrator Tab) host a workspace rail only conditionally — e.g. the
 * selected run has no `work_dir`, or no run is open at all. Such pages broadcast
 * `available: false` so the titlebar can hide its workspace button entirely,
 * matching the conversation/terminal surfaces where the button only appears
 * when a workspace actually exists.
 *
 * Routes that always carry a workspace (conversation / terminal) never need to
 * fire this — the titlebar defaults to available on those routes.
 */
export interface WorkspaceAvailabilityDetail {
  available: boolean;
}

export interface WorkspaceHasFilesDetail {
  hasFiles: boolean;
  conversation_id?: string;
  /**
   * True when this signal corresponds to the workspace tree's first load for
   * this conversation. Lets listeners distinguish backend-seeded files
   * (rules/skills present from the start) from files that appear mid-session.
   *
   * Note: a fresh tree mount counts as initial — switching away from a
   * conversation and back will report `isInitial: true` again, so files added
   * while the conversation was unmounted are not detectable here.
   */
  isInitial: boolean;
}

export function dispatchWorkspaceToggleEvent() {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent(WORKSPACE_TOGGLE_EVENT));
}

export function dispatchWorkspaceStateEvent(collapsed: boolean) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent<WorkspaceStateDetail>(WORKSPACE_STATE_EVENT, { detail: { collapsed } }));
}

/**
 * Declare whether the current route hosts a workspace rail. The orchestrator Tab
 * fires this so the titlebar workspace button hides when no run with a `work_dir`
 * is showing (and reappears when one is). See {@link WorkspaceAvailabilityDetail}.
 */
export function dispatchWorkspaceAvailabilityEvent(available: boolean) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspaceAvailabilityDetail>(WORKSPACE_AVAILABILITY_EVENT, { detail: { available } })
  );
}

/**
 * 当工作空间文件状态变化时触发
 * Dispatch when workspace files status changes
 */
export function dispatchWorkspaceHasFilesEvent(
  hasFiles: boolean,
  conversation_id: string | undefined,
  isInitial: boolean
) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspaceHasFilesDetail>(WORKSPACE_HAS_FILES_EVENT, {
      detail: { hasFiles, conversation_id, isInitial },
    })
  );
}
