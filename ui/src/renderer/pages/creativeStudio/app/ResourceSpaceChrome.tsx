/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useContext, useEffect } from 'react';
import { createPortal } from 'react-dom';

export interface ResourceSpaceChrome {
  section: 'assets' | 'templates';
  headerActions: HTMLElement | null;
  taskActions: HTMLElement | null;
  openTaskCenterOnMount?: boolean;
  onTaskActionsMount?: (mounted: boolean) => void;
}

const ResourceSpaceChromeContext = createContext<ResourceSpaceChrome | null>(null);

export const ResourceSpaceChromeProvider = ResourceSpaceChromeContext.Provider;
export const useResourceSpaceChrome = () => useContext(ResourceSpaceChromeContext);

/** Keep page-owned controls and handlers alive in the shared page header. */
export function ResourceSpaceHeaderActions({ children }: { children: React.ReactNode }) {
  const chrome = useResourceSpaceChrome();
  if (!chrome) return <>{children}</>;
  return chrome.headerActions ? createPortal(children, chrome.headerActions) : null;
}

/** The template task entry stays available from either resource tab. */
export function ResourceSpaceTaskActions({ children }: { children: React.ReactNode }) {
  const chrome = useResourceSpaceChrome();
  const target = chrome?.taskActions;
  const onMount = chrome?.onTaskActionsMount;
  useEffect(() => {
    if (!target) return;
    onMount?.(true);
    return () => onMount?.(false);
  }, [target, onMount]);
  if (!chrome) return <>{children}</>;
  return target ? createPortal(children, target) : null;
}
