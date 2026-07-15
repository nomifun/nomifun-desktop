/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { conversationTarget, isSameSessionTarget, terminalTarget, type SessionTarget } from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import type { WorkspaceTab } from '@/renderer/pages/conversation/Workspace/types';
import {
  WORKSPACE_PANEL_TAB_EVENT,
  type WorkspacePanelTabDetail,
} from '@/renderer/pages/conversation/components/ChatLayout/WorkspaceToolRail';
import { useEffect, useState } from 'react';

const workspacePanelTabStorageKey = (target?: SessionTarget) =>
  target ? sessionStorageKey('workspace-panel-tab', target) : null;

function readStoredTab(target?: SessionTarget): WorkspaceTab {
  const key = workspacePanelTabStorageKey(target);
  if (!key || typeof window === 'undefined') return 'files';
  try {
    return localStorage.getItem(key) || 'files';
  } catch {
    return 'files';
  }
}

export function useWorkspacePanelTabs(target?: SessionTarget): {
  activeWorkspaceTab: WorkspaceTab;
  setActiveWorkspaceTab: (tab: WorkspaceTab) => void;
} {
  const targetKind = target?.kind;
  const targetId = target?.id;
  const stableTarget =
    targetKind === 'conversation' && targetId
      ? conversationTarget(targetId)
      : targetKind === 'terminal' && targetId
        ? terminalTarget(targetId)
        : undefined;
  const [activeWorkspaceTab, setActiveWorkspaceTabState] = useState<WorkspaceTab>(() => readStoredTab(stableTarget));

  useEffect(() => {
    setActiveWorkspaceTabState(readStoredTab(stableTarget));
  }, [targetId, targetKind]);

  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const onTabEvent = (event: Event) => {
      const detail = (event as CustomEvent<WorkspacePanelTabDetail>).detail;
      if (!detail?.target || !stableTarget || !isSameSessionTarget(detail.target, stableTarget)) {
        return;
      }
      const tab = detail?.tab;
      if (tab) setActiveWorkspaceTabState(tab);
    };
    window.addEventListener(WORKSPACE_PANEL_TAB_EVENT, onTabEvent);
    return () => window.removeEventListener(WORKSPACE_PANEL_TAB_EVENT, onTabEvent);
  }, [targetId, targetKind]);

  const setActiveWorkspaceTab = (tab: WorkspaceTab) => {
    setActiveWorkspaceTabState(tab);
    const key = workspacePanelTabStorageKey(stableTarget);
    if (!key) return;
    try {
      localStorage.setItem(key, tab);
    } catch {
      /* ignore storage failures */
    }
  };

  return { activeWorkspaceTab, setActiveWorkspaceTab };
}
