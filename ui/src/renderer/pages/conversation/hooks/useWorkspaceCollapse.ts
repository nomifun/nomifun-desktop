import type { ConversationId } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import {
  conversationTarget,
  isSameSessionTarget,
  terminalTarget,
  type SessionTarget,
} from '@/common/types/ids';
import { blurActiveElement } from '@/renderer/utils/ui/focus';
import {
  WORKSPACE_HAS_FILES_EVENT,
  WORKSPACE_TOGGLE_EVENT,
  dispatchWorkspaceStateEvent,
  type WorkspaceHasFilesDetail,
  type WorkspaceToggleDetail,
} from '@/renderer/utils/workspace/workspaceEvents';
import { useEffect, useRef, useState } from 'react';

type UseWorkspaceCollapseParams = {
  workspaceEnabled: boolean;
  isMobile: boolean;
  /**
   * Legacy render identity whose change forces a mobile collapse. It is not
   * used for event matching or persistence; `target` owns those concerns.
   */
  conversation_id?: ConversationId;
  /** Namespaced session that owns this workspace rail. */
  target?: SessionTarget;
  /**
   * True when the current workspace is an auto-created temporary one (no folder
   * picked by the user). When file-driven auto-expand is enabled, initial temp
   * workspace files are ignored so "send 你好 without picking a folder" leaves
   * the panel collapsed.
   */
  isTemporaryWorkspace?: boolean;
  /**
   * When false, file-tree presence signals do not open the right rail. Manual
   * toggles and persisted manual preferences still work.
   */
  autoExpandOnFiles?: boolean;
};

type UseWorkspaceCollapseReturn = {
  rightSiderCollapsed: boolean;
  setRightSiderCollapsed: React.Dispatch<React.SetStateAction<boolean>>;
  persistRightSiderCollapsed: (collapsed: boolean) => void;
};

type WorkspaceCollapsePreference = 'expanded' | 'collapsed' | null;

type ResolveWorkspaceCollapseAfterHasFilesParams = {
  currentCollapsed: boolean;
  detail: WorkspaceHasFilesDetail;
  isMobile: boolean;
  autoExpandOnFiles: boolean;
  isTemporaryWorkspace?: boolean;
  userPreference: WorkspaceCollapsePreference;
  target?: SessionTarget;
};

export function resolveWorkspaceCollapseAfterHasFiles({
  currentCollapsed,
  detail,
  isMobile,
  autoExpandOnFiles,
  isTemporaryWorkspace,
  userPreference,
  target,
}: ResolveWorkspaceCollapseAfterHasFilesParams): boolean {
  if (!target || !detail.target || !isSameSessionTarget(detail.target, target)) {
    return currentCollapsed;
  }

  if (isMobile) {
    return true;
  }

  if (userPreference) {
    return userPreference === 'collapsed';
  }

  if (!autoExpandOnFiles) {
    return currentCollapsed;
  }

  const isUserPicked = !isTemporaryWorkspace;
  const isMidSession = !detail.isInitial;
  const allowAutoExpand = isUserPicked || isMidSession;
  if (allowAutoExpand && detail.hasFiles) {
    return false;
  }
  if (!detail.hasFiles) {
    return true;
  }
  return currentCollapsed;
}

/**
 * Manages workspace panel collapse/expand state.
 *
 * Default: collapsed. If enabled, file-driven auto-expand fires when
 * WORKSPACE_HAS_FILES_EVENT arrives and either:
 *   - the workspace is user-picked (folder chosen at creation), or
 *   - files appear mid-session in a temporary workspace (e.g. agent writes a
 *     file while the user is here).
 *
 * Manual toggle is persisted under a schema-versioned entity storage key and
 * overrides auto-expand.
 *
 * Known limitation: leaving and re-entering a temporary workspace remounts the
 * workspace tree, so files added while away report as initial load. They will
 * not trigger auto-expand on return — the user must open the panel manually
 * that one time.
 */
export function useWorkspaceCollapse({
  workspaceEnabled,
  isMobile,
  conversation_id,
  target = conversation_id != null ? conversationTarget(conversation_id) : undefined,
  isTemporaryWorkspace,
  autoExpandOnFiles = true,
}: UseWorkspaceCollapseParams): UseWorkspaceCollapseReturn {
  // Workspace panel always starts collapsed; manual toggles and allowed file
  // signals can expand it. See WORKSPACE_HAS_FILES_EVENT handler below.
  const [rightSiderCollapsed, setRightSiderCollapsed] = useState(true);

  // Mirror ref for collapse state
  const rightCollapsedRef = useRef(rightSiderCollapsed);

  const targetKind = target?.kind;
  const targetId = target?.id;
  const stableTarget =
    targetKind === 'conversation' && targetId
      ? conversationTarget(targetId)
      : targetKind === 'terminal' && targetId
        ? terminalTarget(targetId)
        : undefined;
  const preferenceStorageKey = stableTarget ? sessionStorageKey('workspace-collapse', stableTarget) : null;

  useEffect(() => {
    if (!preferenceStorageKey) {
      setRightSiderCollapsed(true);
      return;
    }
    try {
      const stored = localStorage.getItem(preferenceStorageKey);
      setRightSiderCollapsed(stored === 'expanded' ? false : true);
    } catch {
      setRightSiderCollapsed(true);
    }
  }, [preferenceStorageKey]);

  const persistRightSiderCollapsed = (collapsed: boolean) => {
    setRightSiderCollapsed(collapsed);
    if (!preferenceStorageKey) return;
    try {
      localStorage.setItem(preferenceStorageKey, collapsed ? 'collapsed' : 'expanded');
    } catch {
      // ignore errors
    }
  };

  // Keep ref in sync
  useEffect(() => {
    rightCollapsedRef.current = rightSiderCollapsed;
  }, [rightSiderCollapsed]);

  // Listen for workspace toggle events
  useEffect(() => {
    if (typeof window === 'undefined') {
      return undefined;
    }
    const handleWorkspaceToggle = (event: Event) => {
      const detail = (event as CustomEvent<WorkspaceToggleDetail>).detail;
      if (
        !workspaceEnabled ||
        !targetKind ||
        !targetId ||
        !detail?.target ||
        detail.target.kind !== targetKind ||
        detail.target.id !== targetId
      ) {
        return;
      }
      setRightSiderCollapsed((prev) => {
        const newState = !prev;
        if (preferenceStorageKey) {
          try {
            localStorage.setItem(preferenceStorageKey, newState ? 'collapsed' : 'expanded');
          } catch {
            // ignore errors
          }
        }
        return newState;
      });
    };
    window.addEventListener(WORKSPACE_TOGGLE_EVENT, handleWorkspaceToggle);
    return () => {
      window.removeEventListener(WORKSPACE_TOGGLE_EVENT, handleWorkspaceToggle);
    };
  }, [workspaceEnabled, preferenceStorageKey, targetId, targetKind]);

  // Auto expand/collapse workspace panel based on files state (user preference takes priority)
  useEffect(() => {
    if (typeof window === 'undefined' || !workspaceEnabled) {
      return undefined;
    }
    const handleHasFiles = (event: Event) => {
      const detail = (event as CustomEvent<WorkspaceHasFilesDetail>).detail;

      // Check if user has manual preference
      let userPreference: WorkspaceCollapsePreference = null;
      if (preferenceStorageKey) {
        try {
          const stored = localStorage.getItem(preferenceStorageKey);
          if (stored === 'expanded' || stored === 'collapsed') {
            userPreference = stored;
          }
        } catch {
          // ignore errors
        }
      }

      const nextCollapsed = resolveWorkspaceCollapseAfterHasFiles({
        currentCollapsed: rightSiderCollapsed,
        detail,
        isMobile,
        autoExpandOnFiles,
        isTemporaryWorkspace,
        userPreference,
        target: stableTarget,
      });
      if (nextCollapsed !== rightSiderCollapsed) {
        setRightSiderCollapsed(nextCollapsed);
      }
    };
    window.addEventListener(WORKSPACE_HAS_FILES_EVENT, handleHasFiles);
    return () => {
      window.removeEventListener(WORKSPACE_HAS_FILES_EVENT, handleHasFiles);
    };
  }, [
    isMobile,
    workspaceEnabled,
    rightSiderCollapsed,
    isTemporaryWorkspace,
    preferenceStorageKey,
    autoExpandOnFiles,
    targetId,
    targetKind,
  ]);

  // Broadcast workspace state event
  useEffect(() => {
    if (!workspaceEnabled) {
      if (stableTarget) dispatchWorkspaceStateEvent(stableTarget, true);
      return;
    }
    if (stableTarget) dispatchWorkspaceStateEvent(stableTarget, rightSiderCollapsed);
  }, [rightSiderCollapsed, targetId, targetKind, workspaceEnabled]);

  // Force collapse when workspace is disabled
  useEffect(() => {
    if (!workspaceEnabled) {
      setRightSiderCollapsed(true);
    }
  }, [workspaceEnabled]);

  // Mobile: force collapse when entering mobile mode
  useEffect(() => {
    if (!workspaceEnabled || !isMobile || rightCollapsedRef.current) {
      return;
    }
    setRightSiderCollapsed(true);
  }, [isMobile, workspaceEnabled]);

  // Mobile: force collapse workspace on conversation switch to prevent overlay
  useEffect(() => {
    if (!workspaceEnabled || !isMobile) {
      return;
    }
    setRightSiderCollapsed(true);
  }, [conversation_id, isMobile, workspaceEnabled]);

  // Mobile: blur active element on conversation switch to prevent soft keyboard
  useEffect(() => {
    if (!isMobile) {
      return;
    }
    const rafId = requestAnimationFrame(() => {
      blurActiveElement();
    });
    return () => cancelAnimationFrame(rafId);
  }, [conversation_id, isMobile]);

  return { rightSiderCollapsed, setRightSiderCollapsed, persistRightSiderCollapsed };
}
