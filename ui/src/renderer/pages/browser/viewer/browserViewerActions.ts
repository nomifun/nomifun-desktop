/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { BrowserLaneControlState } from '@/common/browser/browserTypes';

export type BrowserViewerSend = (message: Record<string, unknown>) => boolean;

export const requestBrowserViewerTakeover = (send: BrowserViewerSend): boolean =>
  send({ type: 'takeover' });

interface ReturnBrowserViewerControlDependencies {
  invoke: (request: { lane_id: string }) => Promise<unknown>;
  send: BrowserViewerSend;
  refresh: () => Promise<void>;
  setControlState: (state: BrowserLaneControlState) => void;
  setReturningControl: (returning: boolean) => void;
  setViewerError: (error: string | null) => void;
  formatError: (error: unknown) => string;
}

export const returnBrowserViewerControl = async (
  laneId: string,
  dependencies: ReturnBrowserViewerControlDependencies
): Promise<void> => {
  dependencies.setReturningControl(true);
  try {
    await dependencies.invoke({ lane_id: laneId });
    dependencies.setControlState('agent');
    dependencies.send({ type: 'return_control' });
    await dependencies.refresh();
  } catch (error) {
    dependencies.setViewerError(dependencies.formatError(error));
  } finally {
    dependencies.setReturningControl(false);
  }
};
