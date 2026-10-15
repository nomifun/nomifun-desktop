import {
  MIN_CHAT_PANEL_PX,
  MIN_PREVIEW_PANEL_PX,
  MIN_WORKSPACE_PANEL_PX,
} from '@/renderer/pages/conversation/utils/layoutCalc';
import { useEffect } from 'react';

type UseLayoutConstraintsParams = {
  containerWidth: number;
  minChatPanelPx?: number;
  workspaceEnabled: boolean;
  isPreviewOpen: boolean;
  rightSiderCollapsed: boolean;
  setRightSiderCollapsed: React.Dispatch<React.SetStateAction<boolean>>;
};

/**
 * Temporarily collapses the workspace when all three panels cannot fit.
 * Widths and ratios are derived by calcLayoutMetrics, preserving preferences.
 */
export function useLayoutConstraints({
  containerWidth,
  minChatPanelPx = MIN_CHAT_PANEL_PX,
  workspaceEnabled,
  isPreviewOpen,
  rightSiderCollapsed,
  setRightSiderCollapsed,
}: UseLayoutConstraintsParams): void {
  // The workspace yields space without replacing its preferred width.
  useEffect(() => {
    if (!workspaceEnabled || !isPreviewOpen || rightSiderCollapsed) {
      return;
    }
    const safeContainerWidth = Math.max(containerWidth || 0, 1);
    // Auto-collapse workspace when container is too narrow for all three panels
    if (safeContainerWidth < minChatPanelPx + MIN_PREVIEW_PANEL_PX + MIN_WORKSPACE_PANEL_PX) {
      setRightSiderCollapsed(true);
    }
  }, [
    containerWidth,
    minChatPanelPx,
    isPreviewOpen,
    rightSiderCollapsed,
    setRightSiderCollapsed,
    workspaceEnabled,
  ]);

  // The visible chat ratio is clamped synchronously by calcLayoutMetrics.
  // Window resizing must not overwrite the user's saved split preference.
}
