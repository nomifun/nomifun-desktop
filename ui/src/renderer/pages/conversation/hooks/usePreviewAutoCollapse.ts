import { useEffect, useRef } from 'react';

type UsePreviewAutoCollapseParams = {
  isPreviewOpen: boolean;
  workspaceEnabled: boolean;
  rightSiderCollapsed: boolean;
  setRightSiderCollapsed: React.Dispatch<React.SetStateAction<boolean>>;
  siderCollapsed: boolean | undefined;
  setSiderCollapsed: ((value: boolean) => void) | undefined;
  manageSiderCollapse?: boolean;
};

/**
 * Auto-collapses sidebar and workspace when preview opens,
 * restoring their previous state when preview closes.
 */
export function usePreviewAutoCollapse({
  isPreviewOpen,
  workspaceEnabled,
  rightSiderCollapsed,
  setRightSiderCollapsed,
  siderCollapsed,
  setSiderCollapsed,
  manageSiderCollapse = true,
}: UsePreviewAutoCollapseParams): void {
  const previousWorkspaceCollapsedRef = useRef<boolean | null>(null);
  const previousSiderCollapsedRef = useRef<boolean | null>(null);
  const previousPreviewOpenRef = useRef(false);
  const previousSiderManagedRef = useRef(false);

  useEffect(() => {
    if (isPreviewOpen && !previousPreviewOpenRef.current) {
      if (workspaceEnabled && previousWorkspaceCollapsedRef.current === null) {
        previousWorkspaceCollapsedRef.current = rightSiderCollapsed;
      }
      if (workspaceEnabled) setRightSiderCollapsed(true);
    } else if (!isPreviewOpen && previousPreviewOpenRef.current) {
      if (previousWorkspaceCollapsedRef.current !== null) {
        setRightSiderCollapsed(previousWorkspaceCollapsedRef.current);
        previousWorkspaceCollapsedRef.current = null;
      }
    }

    // Browser and Preview share the work pane but use different sidebar
    // policies. Switching between them can keep the pane open throughout.
    const manageSiderNow = isPreviewOpen && manageSiderCollapse;
    if (manageSiderNow && !previousSiderManagedRef.current) {
      if (typeof siderCollapsed !== 'undefined') previousSiderCollapsedRef.current = siderCollapsed;
      setSiderCollapsed?.(true);
    } else if (!manageSiderNow && previousSiderManagedRef.current) {
      if (previousSiderCollapsedRef.current !== null) setSiderCollapsed?.(previousSiderCollapsedRef.current);
      previousSiderCollapsedRef.current = null;
    }
    previousSiderManagedRef.current = manageSiderNow;
    previousPreviewOpenRef.current = isPreviewOpen;
  }, [
    isPreviewOpen,
    siderCollapsed,
    setSiderCollapsed,
    rightSiderCollapsed,
    workspaceEnabled,
    setRightSiderCollapsed,
    manageSiderCollapse,
  ]);
}
