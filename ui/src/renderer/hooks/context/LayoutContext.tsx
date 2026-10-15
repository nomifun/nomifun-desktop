/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import React from 'react';

/** A visible desktop work surface needs room until it closes or unmounts. */
export interface WorkSurfaceSpaceRequest {
  minimumWidth: number;
  close: () => void;
}

export type ReserveWorkSurfaceSpace = (request: WorkSurfaceSpaceRequest) => () => void;

export interface LayoutContextValue {
  siderCollapsed: boolean;
  setSiderCollapsed: (value: boolean) => void;
  reserveSiderSpace?: ReserveWorkSurfaceSpace;
  reserveWorkSurfaceSpace?: ReserveWorkSurfaceSpace;
}

export const LayoutContext = React.createContext<LayoutContextValue | null>(null);

export function useLayoutContext(): LayoutContextValue | null {
  return React.useContext(LayoutContext);
}
